# Complex values and a possible quantum simulator

Status: complex scalar values are implemented. Quantum states, gates and measurement are a future proposal, not language features.

## Why consider quantum simulation?

Probl could combine a small quantum experiment with classical uncertainty about preparation, randomized protocols, measurement outcomes and inference. The existing [BB84 example](../examples/10_quantum_key.probl) encodes the probabilities for an intercept–resend model directly. It does not evolve amplitudes or simulate interference.

A separate state-vector simulator would be a manageable extension. Making arbitrary Probl programs execute as coherent quantum computations would be a much larger language-design project. A useful first experiment would cover Hadamard interference, phase shifts and Bell pairs, before committing to quantum syntax or hardware support.

## The boundary to preserve

Classical alternatives carry nonnegative probability weights. Coherent contributions to the same quantum basis state add as amplitudes, and measurement uses the squared magnitude of the resulting amplitude. A classical mixture of two states is different from their coherent superposition.

Probl's `Weight`, `Dist`, `~`, `if`, `observe` and world merging continue to describe classical probability. In particular, `dist[complex]` means uncertainty about a complex value. Opposite outcomes do not cancel: `one_of([complex(0, 1), complex(0, -1)])` has two outcomes with probability 1/2 each.

The existing engine clears dead variables and merges worlds that agree on their live values. Reusing that rule for amplitudes could create interference when information is merely discarded. Discarding an entangled subsystem requires a partial trace, representable with density matrices or an appropriate classical ensemble. It is not addition of amplitudes based on the remaining variables. Sampling one classical path also does not retain coherent interference between paths.

The [world representation](../crates/probl-engine/src/world.rs), [nonnegative weights](../crates/probl-engine/src/weight.rs) and [finite distributions](../crates/probl-engine/src/dist.rs) therefore remain separate from the [complex scalar kernel](../crates/probl-engine/src/complex.rs).

## Implemented scalar design

- `complex(re, im)` constructs a value with two finite `f64` components. `complex(re)` supplies a zero imaginary component; `complex(z)` also accepts an existing complex value. There is a `complex` type annotation, but annotations do not convert real values: write `complex(1)` explicitly.
- `+`, `-`, unary `-`, `*` and `/` support complex operands, promoting real numeric operands when needed. `z ^ n` supports an `int` exponent, including negative integers; zero to zero is one. Non-integer and complex exponents are deferred until branch conventions are designed.
- `real`, `imag`, `conj`, `abs`, `abs2`, `arg` and `cis` supply the basic scalar operations useful for amplitudes. `cis(theta)` constructs `cos(theta) + i sin(theta)`. `abs2` returns a float, not a probability, and does not normalize anything.
- Complex components and results must be finite; division by zero and overflow are errors. Very small results can underflow. Multiplication and division carry binary exponents separately during intermediate products to avoid spurious overflow and underflow.
- Signed zeros are canonicalized. Equality and hashing remain exact, never tolerance-based; approximate identity checks should use `abs(a - b) < tolerance`. `arg(0)` is zero by convention; a negative real value has argument +pi. Future logarithms and square roots must respect this policy, rather than distinguish signs of zero that world merging treats as equal.
- A purely real complex value compares numerically equal to the corresponding real number with `==`. Storage identity remains typed, as for existing int/float/prob values. Complex numbers have no mathematical ordering; the deterministic internal order for maps, support tables and hashing is not exposed as `<` or `>`.
- Lists, records, maps, bags, functions, finite distributions, `sum` and `mean` accept complex values. The complex mean is a classical weighted arithmetic mean. Reports show complex outcomes as values, without applying real-valued quantiles or summary statistics. Variance and standard deviation remain real-valued APIs in this first version; callers can explicitly query real parts, imaginary parts or magnitudes.
- Existing real-only functions, including `sqrt`, `exp`, `ln`, trigonometry, rounding, distribution parameters and probability checks, do not silently discard an imaginary component or extend their domains. Complex values cannot serve as conditions or weights, even with zero imaginary part.
- There is no new `i` constant or imaginary-literal syntax. Programs can write `let i = complex(0, 1)`. Data-file formats are unchanged; read real and imaginary fields and construct values explicitly.

These choices retain phase and amplitude information as ordinary data, without committing to a representation for quantum registers or adding quantum meaning to general control flow.

## A calculation possible today

This program performs amplitude arithmetic explicitly. No quantum engine is involved:

```probl
fn h(s: list[complex]) -> list[complex] {
    [(s[0] + s[1]) / sqrt(2), (s[0] - s[1]) / sqrt(2)]
}

let start = [complex(1), complex(0)]
let split = h(start)
let shifted = [split[0], split[1] * cis(pi)]
let recombined = h(shifted)

report abs2(recombined[0])   # approximately 0
report abs2(recombined[1])   # approximately 1
```

Changing the phase from pi to zero swaps these outcomes. The amplitudes interfere inside ordinary arithmetic; Probl's world probabilities do not interfere. Applying `h` twice without a phase shift reconstructs the initial state, up to floating-point rounding.

## A possible next layer

An opaque `qstate` could hold a normalized vector of complex amplitudes for a whole register. The first version could offer initialization, H, X, phase rotations, CNOT and final measurement. Gates should preserve normalization; silently renormalizing arbitrary transforms would hide invalid operations and postselection.

Final measurement could return an ordinary `dist` over bit strings. Intermediate measurement would need to return both an outcome and its normalized, collapsed state, so subsequent operations use the right conditional state. Classical uncertainty over quantum states remains a mixture with classical weights. Noise and discarded subsystems require additional mixed-state semantics; they should not be represented by summing state vectors.

Immutable state-vector snapshots fit Probl's value semantics. Copying a classical description for simulation does not implement physical cloning of an unknown qubit. A future hardware-facing API might need separate resource rules.

Dense state vectors use `16 * 2^n` bytes for n qubits with two `f64`s per amplitude: 16 MiB at 20 qubits and 16 GiB at 30, before overhead or classical branching. Any implementation must charge allocations and gate work against host limits. Approximation or amplitude pruning needs an explicit error model; the current unresolved-probability accounting cannot be reused without analysis.

Validation should include H followed by H, phase-dependent interference, Bell correlations, measurement followed by another measurement, preservation of norm, and agreement with an independent simulator. APIs for complex logarithms, global-phase equivalence, generalized measurements, noise and hardware compilation can be designed separately when there is a concrete need.

## Prior art

- [IBM Quantum: single systems](https://learning.quantum.ibm.com/course/basics-of-quantum-information/single-systems) introduces complex state vectors, unitary operations and measurement; [multiple systems](https://learning.quantum.ibm.com/course/basics-of-quantum-information/multiple-systems) covers joint states and entanglement.
- [Cirq simulation](https://quantumai.google/cirq/simulate/simulation) distinguishes pure-state and mixed-state simulators and exposes measurement results separately from simulator state inspection.
- [Q# Bell-pair example](https://learn.microsoft.com/en-us/azure/quantum/qsharp-quickstart) illustrates explicit gate operations and measurement.
- [Python `cmath`](https://docs.python.org/3/library/cmath.html) provides scalar complex operations and documents branch cuts. Its signed-zero conventions differ from Probl's canonical-zero policy; they must not be copied blindly.

Complex arithmetic is useful independently of quantum simulation. Keeping that scalar layer small and explicit allows a quantum engine to be pursued later without changing what a probability or a classical world means.
