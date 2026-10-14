# Collection values: updates, equality and sorting

> Draft, 14 October 2026. Describes how the engine stores and changes lists, maps and bags, as measured at commit `35b4dd4`, and proposes fixes. Nothing proposed is implemented. No program's result changes. Only costs do, and counting more work against the work limit can stop a program that finishes today.

## Recommendation

1. **Change collections in place.** `push`, `pop`, `insert` and `remove` copy the whole collection on every call, so n pushes cost O(n²). They should change it in place when nothing else holds it, as `xs[i] = v` already does, and copy it once when something does. That includes nested places, like `b[0].push("B")`.
2. **Check only what changed.** After a change to a declared variable, the engine checks the whole variable against its type. It only needs to check the new element.
3. **Count comparisons and copies as work.** Comparing two collections, and copying one to change it, aren't counted against the work limit today. String scans are.
4. **Keep `==` as it is.** Don't add an identity operator: whether two equal collections share memory is the engine's business, and a program's result mustn't depend on it.
5. **Keep the merge sort.** Detecting runs that are already in order can come later, if a program needs it.

## How collections are stored

A list is an array behind a reference count, with its hash cached once computed: `Value::List(Arc<Hashed<Vec<Value>>>)` in `crates/probl-engine/src/value.rs`. Maps and bags are stored the same way, over a sorted tree (`BTreeMap`), and so are records. Strings are reference-counted too, without a cached hash. Nothing is copied when a collection is assigned to another variable, passed to a function, stored in another collection, or carried into the worlds a draw splits into. They all share one allocation.

Probl has value semantics: a change through one name is never seen through another.

```probl
let a = ["A"]
var b = [a]
b[0].push("B")
report b               # [["A", "B"]]
report a               # ["A"]
```

In Python or JavaScript, `a` would be `["A", "B"]` too, since `b[0]` and `a` are the same mutable list. Swift's arrays behave like probl's. So changing a collection has to copy what's still shared: here, the inner list, which `a` holds too. Nothing else needs copying.

## Updates

### Today

| Operation | Compiled to | Copies |
|---|---|---|
| `xs[i] = v`, `m[k] = v`, `r.f = v` | An update along the path | A level of the path only if something else holds it |
| `xs.push(v)` | `xs = push(xs, v)` | All of `xs`, every time |
| `xs.pop()` | `t = last(xs)`, then `xs = drop_last(xs)` | All of `xs`, every time |
| `xs.insert(i, v)`, `m.insert(k, v)`, `remove` | `xs = insert(xs, …)` | All of `xs` or `m`, every time |
| `xs = xs + [v]`, `xs += [v]` | A new list | All of `xs` |

Element assignment is copy-on-write (`update` in `crates/probl-engine/src/interp.rs`). The methods are compiled to a call and an assignment (`mutating_method` in `crates/probl-sema/src/lower.rs`), and the built-in copies the collection before changing it.

Measured on a release build, with loops of n changes:

| Loop | n = 10,000 | n = 20,000 | n = 40,000 |
|---|---|---|---|
| `xs.push(i)` | 0.26 s | 0.97 s | 4.0 s |
| `xs.push(i)`, with `xs` declared `list[int]` | 0.73 s | 2.9 s | |
| `b[0].push(i)` | 0.25 s | 0.97 s | |
| `m.insert(i, i)` | 0.88 s | 3.5 s | |
| `xs = xs + [i]` | 0.20 s | 0.78 s | |
| `m[i] = i` | under 0.01 s | 0.01 s | |
| `xs[0] = i` | | | 0.02 s |

Doubling n quadruples the time, except for element assignment, which stays linear. 20,000 assignments to a shared 20,000-element list added no measurable time beyond its one copy.

In `b[0].push("B")`, reading `b[0]` takes a second reference to the inner list. `push` copies the inner list with `"B"` added, and the assignment puts the copy into `b` in place, since only `b` holds the outer list. That's one copy of the inner list, which is needed, because `a` shares it. But a second `b[0].push("C")` copies it again, though nothing else holds it any more.

With a declared type, each of these methods is followed by a check of the whole variable, which is a second pass over it.

### Proposal

A method that changes a collection updates its place directly, as element assignment does. The engine walks the path (`b`, then element 0), copying each level only if something else holds it, then pushes onto what it reaches. In the example, the outer list is never copied, and the inner list is copied on the first push, since `a` shares it, and never after that. n pushes then cost O(n) in total, as appending to an array does in Swift.

- **The method mustn't read the place first.** Reading takes a second reference, and the second reference forces a copy. So this needs its own statement in the IR, not a call followed by an assignment.
- **Whatever else holds a collection makes the next change copy it once.** That could be another variable, another world after a split, or the arguments a cached call keeps. So could the world saved before each statement inside a `try`, kept so that a caught fault resumes from it. There, a change copies the collection once per statement. An undo log would avoid that, but isn't proposed.
- **Only the new element is checked.** The pushed or inserted value is already converted to the declared element type before the change, so nothing else in the variable needs checking. For a nested place like `b[0]`, the element type comes from the variable's declared type.
- **Hashes** of every level on the path are forgotten when it changes, as `Hashed` already does.
- **`xs += [v]`** could append in place in the same way. `xs = xs + [v]` builds a new list, as it does in Swift.

### Alternatives

- **Persistent vectors**, as in Clojure, Scala and Rust's `im` crate, are trees with 32-way branching that share structure. A push costs O(log n) even when the list is shared, so worlds that split and each change a list wouldn't copy all of it. But indexing and iteration slow down by a constant factor, and every built-in that builds a list would change. They're only worth it if changing lists shared by many worlds turns out to be slow.
- **Linked lists**, as in Lisp and Haskell, add at the front in constant time, but index and add at the end in O(n). Probl's lists are indexed.

## Equality

### Today

`==` on two collections goes through `ops::equals`, then `Value::eq`, then `Hashed::eq`, and checks:

1. **Same allocation:** equal, in constant time. That's the case after `var b = a`.
2. **Different hashes, both already computed:** not equal, in constant time. Hashes are computed when worlds merge, and a collection keeps its hash until it changes. `==` doesn't compute one, since that costs as much as comparing.
3. **Otherwise:** it compares the lengths, then the elements in order, stopping at the first difference. Two equal collections in separate allocations always take a full pass, recursively for nested collections.

Equality inside collections is typed, as the [reference semantics](../semantics.md#1-values-and-types) specifies. `1 == 1.0` is true, but `[1] == [1.0]` and `[50%] == [prob(0.5)]` are false. `[1].contains(1.0)` is true. The [annotations proposal](annotations-and-unions.md#open-questions) is affected by this, and tracks it.

### Comparisons aren't counted as work

4,000 comparisons of two equal 200,000-element lists took 2.7 s, with 8 × 10⁸ element comparisons, and none of them counted against the work limit. The copies `push` makes aren't counted either. String scans are counted by their length. So comparing two collections should count one unit for each element compared, and copying one should count one unit for each element copied.

### A constant-time equality

Where other languages have a constant-time equality, it compares identity: JavaScript's `===` on objects, Python's `is`, Java's `==` on objects, Swift's `===` on class instances, and Lisp's `eq`. Structural equality takes a pass everywhere. Python and Swift check identity first, then the elements, as probl does.

An identity operator isn't proposed. Whether two equal collections share memory depends on what the engine did: whether worlds merged, or whether a change copied. It could differ between enumerating and sampling.

Two things could make `==` cheaper without changing any result:

- **Hashes for comparison.** A large collection could compute and keep its hash the first time it's compared. Unequal collections compared again would then take constant time, but equal ones would still take a pass.
- **Hash-consing.** Every collection would be built through a table, so equal collections always share one allocation, and `==` compares pointers. Every construction pays for a lookup, and the table takes memory. Some compilers and decision-diagram libraries do this. It's only worth it for a program that needs it.

## Sorting

### Today

`sort` and `sort_desc`, and `highest` and `lowest` with a comparator, use a bottom-up merge sort written for probl (`try_sort_by` in `crates/probl-engine/src/ordering.rs`).

- **Stable:** equal elements keep their input order, ascending and descending.
- **Cost:** O(n log n) comparisons and a buffer of n elements. Work for n × ⌈log₂ n⌉ steps is counted before sorting starts, then one unit for each comparison.
- **Why not Rust's sort:** a comparator can be a probl function that fails, and the error has to stop the sort and be reported. Rust's sort can't return an error. A comparator can also be inconsistent, like `(a, b) -> 1`. Since Rust 1.81, Rust's sort may panic on that, while a bottom-up merge sort still does a bounded amount of work.
- **Default order:** the order of `<`. Numbers compare by value across int, float and prob. Strings compare by code point, dates chronologically, enum variants in declaration order, and lists lexicographically. Every element is checked first, so a list with an int and a string is an error, even with only two elements.

### Possible improvements

- **Detect runs already in order,** as Timsort does in Python, in Java for objects and in V8, and driftsort in Rust. Sorted or nearly sorted input then costs O(n), and the sort stays stable with bounded work.
- **Copy less.** Each pass copies the whole buffer back. Alternating between the two buffers would avoid that.

Neither is urgent without a program that sorts large lists.

## Implementation order and validation

1. **Changes in place** for `push`, `pop`, `insert` and `remove`, on lists and maps, at nested places too, with only the new element checked. Tests:
   - A benchmark of a loop of pushes, which should take linear time.
   - Tests that value semantics hold: the `a` and `b` example, a list changed in only some worlds after a split, and a saved world that's unchanged when a `try` catches a fault after a change.

   The oracle compares results, not costs, so it keeps checking the semantics.
2. **Count copies and comparisons as work.** Raise the documented limits if a measured program needs it.
3. **Sorting improvements,** if a program needs them.
