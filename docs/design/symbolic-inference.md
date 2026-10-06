# Symbolic exact inference for Probl

> October 4, 2026. Assessment of [Scaling Exact Inference for Discrete Probabilistic Programs](https://arxiv.org/pdf/2005.09089), by Steven Holtzen, Guy Van den Broeck, and Todd Millstein (OOPSLA 2020). This report records an exploratory measurement and proposes an evaluation plan. No symbolic backend has been implemented or benchmarked against Probl.

## Recommendation

Prototype a symbolic backend for a small, finite discrete subset of Probl. Preserve the language's existing semantics and keep the current enumerator, loop solver, analytic continuous inference, and sampling engine available.

The strongest motivation is inference about many related discrete unknowns that must remain available for later queries. Probl's existing optimizations work well when earlier facts become irrelevant and worlds can merge. They do not avoid constructing every combination when many facts remain live together.

This is an inference project, not a reason to add new choice syntax or expose a new kind of boolean. The first milestone should demonstrate a useful performance advantage with ordinary Probl programs and independently checked answers.

## What the paper contributes

Dice compiles discrete probabilistic programs into Boolean formulas represented by binary decision diagrams (BDDs). These graphs share equivalent logical subproblems. Random choices carry separate probability weights; weighted model counting sums the weights of satisfying assignments without explicitly visiting each execution. Counting is linear in the compiled graph's size, though compilation can be expensive.

Its compilation tracks both results and successful observations. For a Boolean query `Q` and evidence `E`, the posterior is `WMC(Q and E) / WMC(E)`. Function compilation can reuse symbolic structure, and multiple queries can reuse a compiled model. These techniques exploit conditional independence and shared logical structure. [Paper, §§2–4 and §5.2](https://arxiv.org/pdf/2005.09089)

The limits matter: BDD size can be exponential, variable ordering strongly affects it, and the paper targets discrete programs with nonrecursive functions and statically bounded iteration. Its benchmark improvements do not establish a speedup over Probl. [Paper, §§5–6 and Appendix D](https://arxiv.org/pdf/2005.09089)

## Where Probl already shares the benefit

| Existing mechanism | What it already avoids | Remaining limitation |
| --- | --- | --- |
| [Liveness and world merging](../../crates/probl-engine/src/world.rs) | Keeping distinct histories after their live states become equal | Distinct live assignments still occupy distinct worlds |
| [Moving draws to their first use](../../crates/probl-sema/src/draws.rs) | Drawing independent values before they are needed | An observation or later query may need many values together |
| [Function-result caching](../../crates/probl-engine/src/interp.rs) | Repeating inference for the same arguments and captured values | Many different argument combinations can require separate cache entries |
| [Solving cyclic loops](../../crates/probl-engine/src/chain.rs) | Repeatedly unrolling certain finite-state cycles | The reachable state space still has to be represented |
| [Analytic continuous values](../../crates/probl-engine/src/analytic.rs) | Enumerating or sampling some affine continuous calculations | This representation does not cover general discrete dependencies |

The [existing benchmarks](../benchmarks.md#since-moving-draws) show that draw scheduling reduced the original reliability example from 1,048,576 worlds to 256. A new backend should be compared with these optimizations enabled. A comparison with naive path enumeration would overstate its incremental value.

## A measured gap: posterior reports keep facts live

Consider independent component failures, followed by evidence that at least one occurred:

```probl
let power ~ bernoulli(1%)
let network ~ bernoulli(1%)
let disk ~ bernoulli(1%)

observe power or network or disk

report power
report network
report disk
```

Each report asks for one component's posterior probability. The program does not request a table of every joint assignment. Nevertheless, all the variables remain live through the observation because they are reported afterwards. Delaying their draws cannot move them past that observation, and merging cannot combine distinct assignments while retaining those facts as concrete values.

During the discussion preceding this report, generated versions with 12, 16, and 18 flags were run against a freshly built release CLI. Each flag had a 1% prior probability, the evidence was their disjunction, and every flag had its own report. Each size was run three times.

| Flags | Peak worlds | World-steps | Median wall time |
| ---: | ---: | ---: | ---: |
| 12 | 4,096 | 57,331 | 11.3 ms |
| 16 | 65,536 | 1,179,631 | 92.0 ms |
| 18 | 262,144 | 5,242,861 | 858.8 ms |

These are exploratory local measurements from October 4, 2026, including process startup, compilation of the Probl source, execution, and output. They are not a portable timing baseline; a revision and hardware inventory were not captured. Peak memory was not measured. The important observation is the exact `2^n` peak world count. No Dice or BDD implementation was run in this experiment.

There is a closed-form reference answer. For independent flags with probabilities `p_i`, and `E` meaning at least one flag is true:

```text
P(E) = 1 - product(1 - p_i)
P(flag_i | E) = p_i / P(E)
```

With equal 1% priors, the posterior of each flag is approximately 8.80164477%, 6.73209233%, and 6.04279854% for the three sizes. The CLI output agreed at its displayed precision. This was an exploratory check, not an automated high-precision regression assertion.

The disjunction and individual flag queries have compact Boolean representations. That makes this a useful initial target, but it also admits a specialized formula. Overlapping alarms and dependent failures are necessary follow-up benchmarks before choosing a general backend.

### Reproducing the experiment

Run from the repository root. The generator writes only temporary files and prints the engine's statistics along with the first few output lines. Timings will vary.

```sh
cargo build --release -p probl-cli
python3 - <<'PY'
import pathlib
import statistics
import subprocess
import tempfile
import time

with tempfile.TemporaryDirectory(prefix="probl-symbolic-review-") as directory:
    for n in (12, 16, 18):
        names = [f"fault_{i}" for i in range(n)]
        source = "\n".join([
            *(f"let {name} ~ bernoulli(1%)" for name in names),
            "observe " + " or ".join(names),
            *(f"report {name}" for name in names),
        ]) + "\n"
        path = pathlib.Path(directory) / f"faults-{n}.probl"
        path.write_text(source, encoding="utf-8")
        times = []
        for _ in range(3):
            start = time.perf_counter()
            result = subprocess.run(
                ["target/release/probl", "run", str(path),
                 "--stats", "--timeout", "15"],
                capture_output=True, text=True, timeout=20,
            )
            times.append(time.perf_counter() - start)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
        print(f"{n} flags: {statistics.median(times):.4f}s median")
        print("\n".join(result.stdout.splitlines()[:4]))
        print(result.stderr)
PY
```

## Capabilities worth pursuing

The following are proposed applications to Probl, not features delivered by the experiment.

**Diagnosis with overlapping evidence.** A service failure can have several possible causes; several alarms can share causes. Users should be able to observe the alarms and report the posterior of each cause without explicitly constructing the joint table. Candidate examples include infrastructure failures, manufacturing defects, and game states inferred from partial observations.

**Many queries about one discrete model.** Separate reports of component failures, combinations of failures, or enum outcomes could share inference work. A model with a huge joint support may still permit cheap individual queries. This does not make printing the whole joint distribution cheap: the requested output itself can be exponential.

**Reusable function structure.** Probl could supplement its cache of concrete function results with compilation reusable across symbolic arguments. The [Dice implementation](https://github.com/SHoltzen/dice#functions) demonstrates the distinction. This would need a contract for captures and fresh random choices at every call; reusing code must never accidentally reuse a draw.

**Parameter exploration.** A later compiled-model facility could let the playground update probabilities while retaining the same logical structure. Reuse would be valid only while the supported outcomes and control structure remain compatible. Changing a probability used as an ordinary numeric value, a loop bound, or a collection size can change that structure. Dependency tracking and cache invalidation would be separate work.

These capabilities complement the [forecast inference proposal](../inference.md). They do not replace the need for better methods for general continuous or non-conjugate posteriors.

## Language contracts to preserve

### Draw identity and branching

The backend must preserve the distinction between recipes and drawn values:

```probl
let d = bernoulli(30%)
let a ~ d
let b = a
let c ~ d

report a == b  # always true
report a == c  # 58%: independent draws
```

An internal symbolic representation of `a` must still behave as a `bool`. `typeof` must not reveal a backend implementation type. Copying a value preserves its identity; drawing again introduces a fresh choice. This applies to repeated calls, captures, probabilistic conditions, and eventually loop iterations.

`chance` must remain branching, and `one_of` must remain distribution construction. Neither needs new syntax to use a symbolic engine. The existing analytic continuous implementation offers an architectural precedent for preserving value identity without immediately materializing every outcome, although its constraint representation cannot simply be reused for discrete logic.

### Observations, errors, and effects

Evidence must affect every subsequent report according to the current contract. Observations inside branches apply only when their branches execute. Impossible evidence must retain Probl's existing behavior; it must not become an arbitrary result after division by zero.

Compilation must also preserve which errors are reachable. An invalid expression in an unexecuted branch must not become an unconditional compilation-time runtime error. Conversely, replacing executions with formulas must not discard errors that the current semantics exposes.

`print` is observable per execution and can prevent optimizations. Exclude it from the first prototype. The initial supported subset should also exclude operations that can fail on symbolic inputs, rather than silently changing when those failures occur.

### Distribution representation

The current finite [distribution type](../../crates/probl-engine/src/dist.rs) contains an explicit vector of outcomes and unresolved probability mass. Keep that implementation detail out of the language contract. A future symbolic distribution should be able to answer supported queries without first converting to this vector.

Enumeration of `support(d)` still requires a collection of results and must obey allocation limits. Querying a probability and materializing all outcomes should have separate internal paths. Likewise, extending `simulate` to return a symbolic joint distribution should be a later milestone; a prototype that answers top-level Boolean reports need not redesign every distribution operation first.

Unresolved mass must remain visible. A backend must never report a normalized, apparently complete answer after silently discarding work that exceeded its limits.

### Integers and finite domains

The [Dice implementation](https://github.com/SHoltzen/dice#unsigned-integers) exposes sized unsigned integers with modular arithmetic. Probl should retain its existing exact integer semantics. A compiler can infer a finite range or encode a finite set of possible values without introducing overflow or wraparound.

Start with booleans. Enums and finite categorical outcomes are a natural next step. Only introduce symbolic integer arithmetic after defining range analysis, intermediate overflow handling within the representation, and the behavior when finite bounds cannot be established. Unsupported cases should retain access to the existing engine.

## Proposed implementation stages

### 1. A bounded Boolean prototype

Accept programs composed of constant-probability Bernoulli draws, Boolean expressions, bindings, conditional branches, Boolean observations, and unconditional top-level Boolean reports. Support deterministic reassignment only if lowering handles it explicitly. Begin with no loops, user calls, `print`, `score`, nested `simulate`, or continuous values.

Build symbolic expressions for values and a shared evidence condition. A query should compute both its posterior and the program's evidence, and it should use Probl's numeric weight discipline. “Exact” here means no Monte Carlo estimation or deliberately omitted outcomes; floating-point rounding still exists.

Keep backend selection experimental. Detect unsupported constructs before execution and use the current engine, or provide an explicit diagnostic in a forced experimental mode. Do not change an exact request into sampling silently. Use node, memory, and work budgets plus cancellation; a graph-size limit is as necessary as today's world limit.

### 2. Measure before expanding the subset

| Benchmark family | Purpose |
| --- | --- |
| Independent flags, one disjunctive observation, every marginal reported | Reproduce the measured gap; check against a formula |
| Overlapping alarms with shared causes | Exercise dependencies that simple draw scheduling cannot remove |
| Conditional failure models | Exercise branch-dependent relationships and guarded evidence |
| Existing reliability and hidden-regime models | Establish the cost on programs already handled efficiently |
| Several reports versus one joint-valued report | Separate query cost from unavoidable output size |
| Different declaration orders and difficult Boolean formulas | Expose graph growth and ordering sensitivity |

Record compilation time, inference time, total time, peak memory, graph nodes, and the current engine's worlds and work. Measure both one query and many queries. Record revision, hardware, toolchain, and repetitions. Include native and actual WASM execution before choosing a dependency or shipping a default.

### 3. Add abstraction and broader values

If the prototype wins on useful models, add user-function compilation with fresh choices and correct captures, enums, and finite distributions. Then evaluate bounded loops and inferred integer domains. General recursion, continuous mixtures, mutation of collections, and composition with the loop solver require separate designs.

Only after this should automatic backend selection or a reusable compiled-model API become a public commitment. A larger supported subset is not worthwhile if transitions between representations routinely materialize the full joint state.

## Verification requirements

Use the [independent rational oracle](../../crates/probl-oracle/src/lib.rs) on its supported finite subset, as well as differential checks against enumeration. Compare evidence and posterior probabilities, not just a few rounded report strings. Extend the generator deliberately where the prototype supports constructs the oracle does not yet cover.

Regression cases should cover shared versus independent draws; captures and repeated calls when introduced; observations inside only one branch; impossible evidence; zero and unit probabilities; many reports; and extremely small nonzero evidence. Relative or log-scale checks are necessary for tiny probabilities: a broad absolute tolerance can incorrectly accept zero.

Also test rejection or fallback for unsupported constructs, cancellation, memory limits, and observable errors. Native/WASM agreement is necessary for portability, but it is not an independent correctness proof. The [testing review](../testing.md) explains the existing oracle's scope and limitations.

## Implementation resources and open questions

[RSDD](https://github.com/neuppl/rsdd) implements binary and sentential decision diagrams in Rust and describes itself as a research project. It is a candidate to evaluate, not a selected dependency. Check weighted-counting APIs, ownership and reclamation, cancellation, numeric weights, maintenance, and WASM support against the prototype's needs. Do not infer browser suitability merely from the implementation language.

The difficult integration decisions are:

- How to lower branches and assignments while preserving guarded evidence and errors.
- How to represent symbolic values and captures without changing user-visible identity.
- Which variable-ordering strategy works for Probl's target models.
- How to share work among reports without forcing full joint output.
- Where symbolic execution ends and the current engines resume, without losing correlations or evidence.

The earlier [benchmark assessment](../benchmarks.md) correctly favored cheaper changes for its workloads. Its claim that large game states cannot benefit from symbolic methods was too categorical: large state counts alone do not prove that no compact representation exists. The new posterior-reporting example justifies a focused experiment. It does not yet justify replacing the existing inference engine.
