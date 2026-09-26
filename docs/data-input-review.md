**Review of the data-input proposal**

Reviewed against `9a47954`, 26 September 2026. This is a design review of [data-input.md](data-input.md), checked against the current compiler and runtime. The input feature is not implemented, so the scenarios below identify missing contracts rather than reproduced loader bugs.

The overall direction is sound: declared schemas control interpretation, inputs are loaded before simulation, and the engine receives values without filesystem access. Keep those decisions. Six issues should be resolved before implementation; the most important prerequisite is that the current IR does not retain enough type information to decode the examples.

**1. High — Retain a complete, resolved data schema before handing inputs to the loader.**

The [implementation section](data-input.md#L162) records each source's “type” and then has the engine decode it. However, today's [record IR](../crates/probl-sema/src/ir.rs#L105) stores only field names. [Type-declaration lowering](../crates/probl-sema/src/lower.rs#L581) discards field types, and [runtime conformance](../crates/probl-engine/src/interp.rs#L826) checks a named record's tag rather than recursively checking its fields.

Consequently, `list[Day]` in the proposed source manifest cannot currently tell the loader that `day` is a date and `signups` is an integer. The nested `Assumptions` example needs recursively resolved record and enum definitions. Checking only the outer type also cannot reject a record that contains a forbidden `dist[int]` field.

**Recommendation:** explicitly add a resolved schema representation to the compiler work. Retain field types and spans, resolve named references, and validate the entire reachable schema before opening a source. Define cycle handling. Reuse these definitions for boundary validation and the eventual static checker. Reject unsupported format/type combinations during compilation: CSV's `list[R]` rule still needs to specify whether compound fields are unsupported or have a defined cell encoding.

Treat loaded inputs as validated bindings tied to the compiled program, rather than an unqualified vector of values in `Options`. This prevents hosts from accidentally supplying missing, reordered, or schema-incompatible values and makes input overrides easier to add later.

**2. High for hosted use — Literal paths do not establish read authority.**

The [literal-path rule](data-input.md#L69) makes dependencies discoverable, but does not constrain what the CLI or another host may open. Absolute paths, parent traversal, symlinks outside a permitted directory, and special files remain unspecified. Keeping filesystem access outside the engine is useful separation, but an unrestricted host resolver can still expose the host's files through reports or value-bearing diagnostics.

This matters to the project's stated support for untrusted models; it does not mean a trusted local CLI must forbid arbitrary user files.

**Recommendation:** define a host input resolver with an explicit policy. The compiled program requests sources; the host grants specific byte streams or uploaded files. A hosted implementation should have no ambient filesystem access through model paths. A local CLI can deliberately use a broader policy, with special files and stdin handled explicitly. If directory confinement is offered, enforce it when opening files, including symlink handling; rejecting `..` alone is insufficient.

The same resolver and authorization policy must apply to `run`, data validation, and any future editor integration. Input permission belongs to the host and cannot be widened by a model.

**3. High for hosted use — Bound ingestion and decoded allocations, not just file size.**

The [limits section](data-input.md#L149) lists a 64 MiB input limit, collection length, and JSON depth. It does not specify whether the byte limit is per source or aggregate, when it is enforced, or how it covers decoded values. The existing [collection limit](../crates/probl-engine/src/lib.rs#L36) is per collection, not an aggregate memory limit.

Many individually acceptable files can exceed a host's intended budget. A short JSON array can allocate much larger runtime objects; a large array of small records can remain below each individual collection limit. A full byte buffer, generic JSON tree, converted runtime values, and diagnostic storage may coexist. Checking the limit after `read_to_end` or after materializing those structures is too late. An input stream can also stall without exceeding its byte limit.

**Recommendation:** specify aggregate input bytes and decoded nodes/bytes, plus per-source and per-value limits where useful. Charge budgets while reading and before allocation. Apply cancellation and a host-controlled deadline to loading as well as simulation. Validate the whole input set before launching workers, then share immutable values between batches. Apply the same limits to `probl schema`; inference of a schema must not be an unbounded alternate parser path. Avoid unnecessary intermediate trees, but do not require a streaming simulation API in this version.

**4. Medium — Name and key conversion need collision checks in both directions.**

The [name rule](data-input.md#L86) catches a declared field matching multiple input names. It does not catch multiple declared fields matching the same input name:

```probl
type Row = { ab: int, a_b: int }
let rows: list[Row] = read("rows.csv")
```

With one CSV header `AB`, each field individually has one match. The same value would populate two logically distinct fields unless the schema rejects this collision.

JSON introduces two additional cases. Duplicate object keys can be lost if the parser first constructs a map. Typed map keys can collide *after* conversion: if both spellings are accepted as integers, `{"1": "first", "01": "second"}` cannot faithfully become a `map[int, str]`. RFC 8259 recommends unique object names but describes differing implementation behavior for duplicates; merely citing the RFC does not choose Probl's policy. [RFC 8259, section 4](https://www.rfc-editor.org/rfc/rfc8259#section-4).

**Recommendation:** reject normalized collisions among declared record fields and ambiguous matches among input names. Reject duplicate decoded JSON keys before constructing a map, and reject duplicate typed map keys after conversion. Specify the supported JSON object-key types; arbitrary composite `K` values have no encoding in the current proposal. Restrict object-form bags similarly, while retaining array-form bags for compound items. Record-field name normalization should not silently normalize ordinary `map[str, V]` keys.

Keep the approved case/punctuation-insensitive matching, but make its normalization algorithm deterministic and use exactly the same implementation in loading and `schema`.

**5. Medium — Keep ordinary `check` independent of runtime input streams.**

The [CLI plan](data-input.md#L169) makes `probl check` read the data too. Combined with [stdin inputs](data-input.md#L72), this can block an editor or CI check waiting for EOF, consume a pipe intended for a subsequent run, and prevent checking a valid model on a machine without its datasets. The current command only compiles the program. This would be a significant tooling contract change.

**Recommendation:** retain a static `probl check`; add explicit data validation, such as `probl check --data`, using the same loader as `run`. `run` should always load and validate its actual inputs before executing. A previous successful validation cannot guarantee that a mutable file is still the same.

Specify snapshot lifetime too. Repeated references to one source during a run should use the same loaded bytes, even if they decode under different schemas. Existing REPL bindings should retain their input snapshots until explicitly reloaded; the current REPL reruns the accumulated program, so blindly reloading all sources would silently alter earlier bindings. Store input identity and a content digest with run metadata from the first version, even if displaying that metadata remains deferred.

**6. Medium — The custom-parser constraint hides substantial semantic work.**

The [implementation plan](data-input.md#L166) requires new CSV and JSON parsers with no new dependency, while estimating roughly 1,000 lines including tests. That estimate omits decisions with direct consequences for data integrity:

- Integer data must not pass through `f64` first: `9007199254740993` fits Probl's `i64`, but rounding through binary64 loses its identity. Define integer range checks, finite float/probability requirements, and rejection of out-of-range exponents. JSON allows implementations to impose numeric range and precision limits. [RFC 8259, section 6](https://www.rfc-editor.org/rfc/rfc8259#section-6).
- The proposed quoted/unquoted CSV trimming makes quoting affect string values. ` ACME ` and `" ACME "` decode differently even though their field contents are the same under RFC 4180. Quoting changes by an export tool can therefore change joins or category identities. RFC 4180 explicitly preserves spaces in fields. [RFC 4180, section 2](https://www.rfc-editor.org/rfc/rfc4180#section-2).
- Duplicate detection and precise source positions must survive tokenization. A generic JSON object map or CSV API that discards quotation metadata may already have erased information required by the proposal.

**Recommendation:** remove “no new dependency” as a design requirement. Evaluate established tokenizers such as [`csv`](https://docs.rs/csv/latest/csv/struct.ReaderBuilder.html) and [`serde_json`](https://docs.rs/serde_json/latest/serde_json/), keeping Probl's schema conversion, strictness, and budgets in its own layer. Verify their behavior against the required dialect; defaults need not provide every guarantee. Preserve string contents regardless of quoting, and define any numeric/date whitespace tolerance at conversion time. If the custom trimming rule is retained, document it as a deliberate dialect difference.

If parsers are intentionally written in-house, budget for malformed-input fuzzing, conformance cases, differential tests, and resource-limit tests. Library choice is flexible; tested decoding behavior is the requirement.

**The two open questions**

- **Keep `read` as the initial syntax**, but compile it into an input manifest with stable identities, resolved schemas, and default locations. Hosts can bind inputs through that manifest now; a `--input pilot=...` override can follow without changing model semantics. Named-input syntax does not need to block this release. [CmdStan's declared data and external JSON inputs](https://mc-stan.org/docs/cmdstan-guide/json_apdx.html) are useful prior art for keeping model definitions separate from supplied values.
- **Resolve relative paths against the model's directory.** This makes a model/data bundle portable. Give in-memory programs an explicit host-provided base or virtual resolver. Document whether a symlinked model uses its invocation location or target location. The REPL's initial working directory is a reasonable base, with the snapshot behavior above.

Before implementation, add an acceptance matrix covering: nested named schemas and forbidden nested types; denied source resolution; aggregate limits and interrupted streams; normalized-field and converted-key collisions; large integer preservation; static checking without datasets; and reuse of one input snapshot across sampling batches and REPL evaluations. These are the boundaries most likely to produce architectural rework or plausible but incorrect model inputs.

No implementation changes or runtime tests were made for this review: `read` is still a proposal. Source references and format specifications were inspected against the current checkout.
