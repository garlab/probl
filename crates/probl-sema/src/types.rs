//! Canonical unions and their recursive declaration rules.
use crate::ir::{RecordType, TypeSpec};

impl TypeSpec {
    pub fn union(types: impl IntoIterator<Item = Self>) -> Self {
        let mut members = Vec::new();
        for ty in types {
            match ty {
                Self::Union(xs) => members.extend(xs),
                t => members.push(t),
            }
        }
        members.sort();
        members.dedup();
        if members.len() == 1 {
            members.pop().unwrap()
        } else {
            Self::Union(members)
        }
    }

    /// Exact scalar membership, including a member of a canonical union.
    pub fn includes(&self, ty: &Self) -> bool {
        self == ty || matches!(self, Self::Union(xs) if xs.contains(ty))
    }

    pub fn container_kind(&self) -> Option<&'static str> {
        Some(match self {
            Self::List(_) | Self::BareList => "list",
            Self::Set(_) | Self::BareSet => "set",
            Self::Bag(_) | Self::BareBag => "bag",
            Self::Map(..) | Self::BareMap => "map",
            Self::Dist(_) => "dist",
            _ => return None,
        })
    }

    pub fn member_kind(&self, kind: &str) -> Option<&Self> {
        match self {
            Self::Union(xs) => xs.iter().find(|t| t.container_kind() == Some(kind)),
            t => (t.container_kind() == Some(kind)).then_some(t),
        }
    }

    /// A static context exists when all applicable members agree on it.
    pub fn project(&self, select: impl Fn(&Self) -> Option<Self> + Copy) -> Option<Self> {
        match self {
            Self::Union(xs) => {
                let mut types = xs.iter().filter_map(select);
                let first = types.next()?;
                types.all(|t| t == first).then_some(first)
            }
            t => select(t),
        }
    }
}

/// Each position is independent; named records can be recursive.
pub fn validate(ty: &TypeSpec, records: &[RecordType]) -> Vec<(String, TypeSpec, TypeSpec)> {
    fn fields(ty: &TypeSpec, records: &[RecordType]) -> Option<Vec<String>> {
        match ty {
            TypeSpec::Record(r) => Some(
                records[*r as usize]
                    .fields
                    .iter()
                    .map(|f| f.name.clone())
                    .collect::<Vec<_>>(),
            ),
            TypeSpec::AnonRecord(fs) => Some(fs.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>()),
            _ => None,
        }
        .map(|mut fs| {
            fs.sort();
            fs
        })
    }
    fn walk(
        ty: &TypeSpec,
        records: &[RecordType],
        seen: &mut [bool],
        path: &str,
        errors: &mut Vec<(String, TypeSpec, TypeSpec)>,
    ) {
        let mut child = |t: &TypeSpec, name: &str| {
            let path = if path.is_empty() {
                name.to_owned()
            } else {
                format!("{path}: {name}")
            };
            walk(t, records, seen, &path, errors);
        };
        match ty {
            TypeSpec::List(t) => child(t, "list element"),
            TypeSpec::Set(t) => child(t, "set element"),
            TypeSpec::Bag(t) => child(t, "bag element"),
            TypeSpec::Map(k, v) => {
                child(k, "map key");
                child(v, "map value");
            }
            TypeSpec::Dist(t) => child(t, "distribution outcome"),
            TypeSpec::AnonRecord(fs) => {
                for (n, t) in fs {
                    child(t, &format!("field `{n}`"));
                }
            }
            TypeSpec::Record(r) => {
                if !std::mem::replace(&mut seen[*r as usize], true) {
                    for f in &records[*r as usize].fields {
                        let field = format!("{}.{}", records[*r as usize].name, f.name);
                        let next = if path.is_empty() {
                            field
                        } else {
                            format!("{path}: {field}")
                        };
                        walk(&f.ty, records, seen, &next, errors);
                    }
                }
            }
            TypeSpec::Union(xs) => {
                let mut kinds = std::collections::BTreeMap::new();
                let mut shapes = std::collections::BTreeMap::<Vec<String>, (&TypeSpec, Option<&TypeSpec>)>::new();
                for a in xs {
                    if let Some(kind) = a.container_kind() {
                        if let Some(b) = kinds.insert(kind, a) {
                            errors.push((path.to_owned(), b.clone(), a.clone()));
                        }
                    }
                    if let Some(shape) = fields(a, records) {
                        let anonymous = matches!(a, TypeSpec::AnonRecord(_));
                        if let Some((first, anon)) = shapes.get_mut(&shape) {
                            if let Some(b) = if anonymous { Some(*first) } else { *anon } {
                                errors.push((path.to_owned(), b.clone(), a.clone()));
                            }
                            if anonymous {
                                *anon = Some(a);
                            }
                        } else {
                            shapes.insert(shape, (a, anonymous.then_some(a)));
                        }
                    }
                    walk(a, records, seen, path, errors);
                }
            }
            _ => {}
        }
    }
    let mut errors = Vec::new();
    walk(ty, records, &mut vec![false; records.len()], "", &mut errors);
    errors
}
