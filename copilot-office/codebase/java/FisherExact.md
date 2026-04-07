# FisherExact

**Source**: `data/fishertest/FisherExact.java`, `data/fishertest/UnirootZeroIn.java`
**LOC**: 301
**Pipeline Stage**: Output metrics for strand bias and somatic significance
**Rust counterpart**: not yet ported in this workspace
**Status**: complete

## Overview

`FisherExact` is the Java-side statistical helper that converts a 2x2 contingency table into Fisher exact p-values and a conditional maximum-likelihood odds ratio. It is instantiated only in the output-printer layer, but it is parity-critical because those printed values are visible outputs in simple, somatic, and amplicon modes. `UnirootZeroIn` is a single-purpose numerical helper used only by `FisherExact.mle()` to solve for the non-centrality parameter whose expected table count matches the observed count. Together they implement VarDictJava's replacement for the older R-based Fisher path: one-sided and two-sided p-values are computed from a central hypergeometric distribution, while the odds ratio is computed by inverting the mean of the non-central hypergeometric distribution.

## Statistical Model And Pipeline Role

The input table is:

$$
\begin{array}{c|cc}
 & \text{Forward} & \text{Reverse} \\
\hline
\text{Reference} & refFwd & refRev \\
\text{Alternate} & altFwd & altRev
\end{array}
$$

`FisherExact` rewrites that table into the standard hypergeometric margins:

- $m = refFwd + refRev$ = total reference reads
- $n = altFwd + altRev$ = total alternate reads
- $k = refFwd + altFwd$ = total forward-strand reads
- $x = refFwd$ = observed upper-left cell
- $lo = \max(0, k - n)$ and $hi = \min(k, m)$ = admissible support for $x$

The output-printer layer uses those statistics in three ways:

1. `SimpleOutputVariant` computes one Fisher test per emitted row for single-sample strand bias.
2. `AmpliconOutputVariant` computes the same strand-bias summary for the representative amplicon call.
3. `SomaticOutputVariant` computes three Fisher tests: tumor strand bias, normal strand bias, and a tumor-vs-normal significance table based on variant vs non-variant counts.

`PostProcessModules` do not instantiate `FisherExact` directly. Their parity impact is indirect but important: they choose which `Variant`, reference placeholder, or `null` object is passed into the output-variant constructors, which changes the contingency-table counts that `FisherExact` sees.

## Method Inventory

### FisherExact.java

| Method | Lines | Analyzed? | Summary |
|--------|-------|-----------|---------|
| `FisherExact(int, int, int, int)` | L41-L55 | yes | Builds the contingency-table margins, support, cached central log-probability vector, and eager p-values. |
| `logdcDhyper(int, int, int)` | L58-L75 | yes | Caches rounded central hypergeometric log-probabilities over the support. |
| `mle(double)` | L85-L101 | yes | Solves for the conditional MLE odds ratio using `UnirootZeroIn`. |
| `mnhyper(Double)` | L103-L115 | yes | Computes the expectation of the non-central hypergeometric distribution. |
| `dnhyper(Double)` | L117-L134 | yes | Computes the normalized non-central hypergeometric mass over the support. |
| `getOddRatio()` | L136-L145 | yes | Formats the conditional MLE odds ratio for output. |
| `getPValue()` | L147-L149 | yes | Returns the rounded two-sided p-value. |
| `getLogdc()` | L151-L154 | yes | Recomputes and returns the cached central log-probability vector. |
| `getPValueGreater()` | L156-L158 | yes | Returns the rounded upper-tail p-value. |
| `getPValueLess()` | L160-L162 | yes | Returns the rounded lower-tail p-value. |
| `round_as_r(double)` | L164-L169 | yes | Reproduces R-like display rounding at 5 decimal places. |
| `calculatePValue()` | L171-L184 | yes | Computes lower-tail, upper-tail, and two-sided p-values for the observed table. |
| `pnhyper(int, boolean)` | L186-L197 | yes | Delegates one-tailed cumulative probability to Apache Commons Math. |

### UnirootZeroIn.java

| Method | Lines | Analyzed? | Summary |
|--------|-------|-----------|---------|
| `zeroinC(double, double, Function<Double, Double>, double)` | L22-L101 | yes | Hybrid inverse-interpolation and bisection root finder used for Fisher odds-ratio inversion. |

## Method Analyses

### FisherExact(int refFwd, int refRev, int altFwd, int altRev)

**Purpose**: Convert strand-specific coverage counts into the hypergeometric parameterization used throughout the rest of the class.

**Algorithm**:

1. Collapse the 2x2 table into row and column totals: `m`, `n`, `k`, and observed cell `x`.
2. Compute the legal support of the upper-left cell as `lo = max(0, k - n)` and `hi = min(k, m)`.
3. Materialize `support` as every integer from `lo` through `hi`, inclusive.
4. Call `logdcDhyper(m, n, k)` once and store the result in `logdc`.
5. Call `calculatePValue()` immediately so all p-value fields are ready before any getter is called.

**Parity note**: There is no input validation here. Callers are assumed to provide non-negative counts that form a meaningful contingency table.

### logdcDhyper(int m, int n, int k)

**Purpose**: Build the reusable cache of central hypergeometric log-probabilities.

**Algorithm**:

1. Allocate a new local `logdc` list.
2. Iterate every admissible cell value in `support`.
3. If `m + n == 0`, append `0.0` and continue. This gives the all-zero table a degenerate support with log-mass `0`.
4. Otherwise instantiate `HypergeometricDistribution(m + n, m, k)`.
5. Evaluate `logProbability(element)` for the current support point.
6. If the library returns `NaN`, replace it with `0.0`.
7. Round the log-probability with `roundHalfEven("0.0000000", value)` before caching it.
8. Return the full list after the support is exhausted.

**What is actually cached**:

- Despite the field name `logdc`, this is not a log-factorial cache.
- The class caches the rounded central log PMF value for each support point.
- The cached vector is reused as the base term in `dnhyper()` when a non-centrality parameter is applied.

### mle(double x)

**Purpose**: Compute the conditional MLE of the odds ratio, represented as the non-centrality parameter of Fisher's non-central hypergeometric model.

**Algorithm**:

1. Set `eps = Math.ulp(1.0)`.
2. If the observed cell is the lower support boundary, return `0.0` immediately.
3. If the observed cell is the upper support boundary, return `Double.POSITIVE_INFINITY` immediately.
4. Compute the expected cell count under the null odds ratio of `1.0` with `mnhyper(1.0)`.
5. If the null expectation is larger than the observation, solve `mnhyper(t) - x = 0` on `[0, 1]` with `UnirootZeroIn.zeroinC(...)`.
6. If the null expectation is smaller than the observation, solve `mnhyper(1 / t) - x = 0` on `[eps, 1]`, then invert the root to map it back above `1`.
7. If the null expectation already equals the observation, return `1.0`.

**Why the reciprocal branch exists**:

- The root finder is only asked to search within a bounded interval ending at `1`.
- Odds ratios above `1` are represented by solving for `t` below `1` and then returning `1 / t`.

**Root-finder tolerance**: The tolerance passed to `zeroinC` is `eps^(1/4)`, not `eps` or `sqrt(eps)`.

### mnhyper(Double ncp)

**Purpose**: Compute the expectation of the non-central hypergeometric distribution at a given odds ratio.

**Algorithm**:

1. If `ncp == 0`, return `lo` directly.
2. If `ncp` is infinite, return `hi` directly.
3. Otherwise call `dnhyper(ncp)` to get the normalized probability mass over the support.
4. Multiply each support value by its probability.
5. Sum the products and return the result.

This expectation is the monotone function that `mle()` inverts.

### dnhyper(Double ncp)

**Purpose**: Convert the central cached distribution into a non-central hypergeometric distribution for a specific odds ratio.

**Algorithm**:

1. Start a new `result` list.
2. For each support index `i`, compute `logdc[i] + log(ncp) * support[i]`.
3. Find the maximum transformed log-weight.
4. Exponentiate each log-weight after subtracting the maximum. This is a standard stability trick that avoids overflow.
5. Sum those exponentiated values.
6. Normalize each exponentiated value by the sum and return the normalized list.

**Numerical behavior**:

- `dnhyper()` assumes `ncp > 0` and finite. The `ncp == 0` and `ncp == Infinity` branches are handled earlier by `mnhyper()`.
- Because `logdc` was already rounded to 7 decimal places, the non-central distribution starts from a rounded, not exact, central baseline.

### getOddRatio()

**Purpose**: Convert the conditional MLE into the string format used in TSV output.

**Algorithm**:

1. Call `mle(x)` using the observed upper-left cell.
2. If the result is infinite, return the literal string `"Inf"`.
3. If the result is exactly equal to `Math.round(result)`, format it as an integer with `DecimalFormat("0")`.
4. Otherwise round it with `round_as_r(...)` and convert that rounded double to a string.

**Confidence-interval caveat**: This method exposes only the point estimate. Unlike R's `fisher.test`, this Java implementation does not compute confidence intervals.

### getPValue()

Returns `PvalueTwoSided` after passing it through `round_as_r(...)`. The stored field is raw double precision; the public API is rounded to 5 decimal places.

### getLogdc()

**Purpose**: Expose the current central log-probability vector.

**Algorithm**:

1. Recompute `logdc` by calling `logdcDhyper(m, n, k)` again.
2. Replace the field with the recomputed list.
3. Return the new list.

**Parity note**: This is not a pure getter. It mutates the cached field on every call.

### getPValueGreater()

Returns `PvalueGreater` after `round_as_r(...)`. `SomaticOutputVariant` compares this rounded value against the rounded lower-tail value when deciding which one-tailed p-value to print.

### getPValueLess()

Returns `PvalueLess` after `round_as_r(...)`. The comparison in somatic mode therefore happens after 5-decimal rounding, not on the raw probabilities.

### round_as_r(double value)

**Purpose**: Reproduce the 5-decimal output style VarDict expects from the older R path.

**Algorithm**:

1. Multiply the input by `RESULT_ROUND_R` (`1E5`).
2. Round with `roundHalfEven("0", scaledValue)`.
3. Divide back by `RESULT_ROUND_R`.
4. Canonicalize exact `0.0` to `0` and exact `1.0` to `1`.
5. Return the rounded value.

`RESULT_ROUND_R` is a mutable `public static` field, so tests or other code could change rounding globally.

### calculatePValue()

**Purpose**: Populate all p-value fields once the table and cache are initialized.

**Algorithm**:

1. Compute the lower-tail p-value with `pnhyper(x, false)` and store it in `PvalueLess`.
2. Compute the upper-tail p-value with `pnhyper(x, true)` and store it in `PvalueGreater`.
3. Set `relErr = 1 + 1E-7`.
4. Build the central distribution `d = dnhyper(1.0)`.
5. Look up the observed support point's probability as `d.get(x - lo)`.
6. Sum every central probability `el` such that `el <= observedProbability * relErr`.
7. Store that sum as `PvalueTwoSided`.

**Important semantic point**:

- The two-sided p-value is not `2 * min(P_less, P_greater)`.
- It is the R-style sum of all tables whose probability is less than or equal to the observed table's probability, with a small relative-error cushion.

### pnhyper(int q, boolean upper_tail)

**Purpose**: Compute one-sided central hypergeometric tail probabilities.

**Algorithm**:

1. If `m + n == 0`, return `1.0` immediately.
2. Otherwise instantiate `HypergeometricDistribution(m + n, m, k)`.
3. If `upper_tail` is true, return `upperCumulativeProbability(q)`.
4. Otherwise return `cumulativeProbability(q)`.

This method is called only during eager p-value initialization.

### UnirootZeroIn.zeroinC(double ax, double bx, Function<Double, Double> f, double tol)

**Purpose**: Find a root of `f` inside a bracket using the classic `zeroin` hybrid strategy.

**Algorithm**:

1. Initialize `a = ax`, `b = bx`, and `c = a`.
2. Evaluate `fa = f(a)`, `fb = f(b)`, and `fc = fa`.
3. Enter an unbounded iteration loop.
4. Compute `prev_step = b - a` and a default `new_step = (c - b) / 2.0`, which is the bisection fallback.
5. If `|fc| < |fb|`, rotate `(a, b, c)` and `(fa, fb, fc)` so `b` remains the best current approximation.
6. Compute the active tolerance as `2 * EPSILON * |b| + tol / 2.0`.
7. If the proposed half-interval step is already within tolerance, or `fb == 0.0`, return `b`.
8. If the previous step was large enough and `|fa| > |fb|`, try interpolation instead of pure bisection.
9. If only two distinct points remain (`a == c`), use linear interpolation.
10. Otherwise use inverse quadratic interpolation based on `(a, b, c)` and `(fa, fb, fc)`.
11. Normalize the sign convention so `p` is positive and the sign is carried by `q`.
12. Accept the interpolated step only if it stays safely inside the bracket and is not too large compared with the previous step.
13. If the accepted step is smaller than the active tolerance, force it to exactly `+tol_act` or `-tol_act`.
14. Shift `a <- b`, `fa <- fb`, move `b += new_step`, and evaluate `fb = f(b)`.
15. If `fb` and `fc` now have the same sign, collapse the bracket by setting `c = a` and `fc = fa`.
16. Repeat until the termination condition in step 7 succeeds.

**Behavioral notes**:

- There is no explicit iteration cap.
- The algorithm relies on the caller providing a monotone, bracketed problem. In this module that caller is `FisherExact.mle()`.

## Confidence Intervals

R's `fisher.test` is often discussed together with an odds-ratio confidence interval obtained by inverting one-sided tests. This Java implementation stops earlier. It exposes:

- one lower-tail p-value
- one upper-tail p-value
- one R-style two-sided p-value
- one conditional MLE odds ratio

It does **not** expose lower or upper confidence bounds, and `UnirootZeroIn` is used only for the point estimate. A Rust port that adds confidence-interval calculations would be extending behavior, not matching existing Java output.

## Cross-Module Dependencies

- Depends on:
  - `org.apache.commons.math3.distribution.HypergeometricDistribution` for central hypergeometric log-probabilities and cumulative tails.
  - `Utils.roundHalfEven(...)` for Java-side banker's rounding that approximates R's output formatting.
  - `UnirootZeroIn.zeroinC(...)` for odds-ratio inversion.
- Called by:
  - `SimpleOutputVariant` for single-sample strand-bias output.
  - `AmpliconOutputVariant` for amplicon strand-bias output.
  - `SomaticOutputVariant.calculateFisherSomatic(...)` for tumor strand bias, normal strand bias, and somatic significance.
- Indirectly shaped by:
  - `PostProcessModules`, which choose whether the printers receive a real `Variant`, a reference placeholder, or `null`, thereby changing the contingency table seen here.

## Known Parity Traps

- `logdc` is a cache of rounded central log PMF values over the support, not a cache of `log(n!)` terms. Porting it as factorial memoization would change both rounding points and reuse semantics.
- `logdcDhyper()` rounds every cached log-probability to 7 decimals before `dnhyper()` uses it. The odds ratio and two-sided p-value therefore depend on a rounded cache, not raw library values.
- `getLogdc()` recomputes and mutates the cache instead of returning the existing list.
- `calculatePValue()` uses the R-style two-sided definition based on probability ordering with `relErr = 1 + 1E-7`; it is not equivalent to doubling the smaller one-tailed p-value.
- `getPValueLess()` and `getPValueGreater()` round to 5 decimals before `SomaticOutputVariant` compares them, so somatic mode chooses the smaller rounded tail, not necessarily the smaller raw tail.
- `getOddRatio()` returns the string `"Inf"` for the upper boundary case and never returns a numeric infinity token.
- All-zero tables (`0, 0, 0, 0`) produce lower-tail, upper-tail, and two-sided p-values of `1`, while the odds ratio becomes `0` because the observed cell equals the lower support bound.
- `RESULT_ROUND_R` is mutable global state. Any test or caller that changes it will affect every future Fisher output in the JVM.
- `logdcDhyper()` converts `NaN` log-probabilities to `0.0`, which effectively injects unit-weight log mass instead of propagating the invalid value.
- Runtime and memory are linear in support size `hi - lo + 1`. Deep coverage increases both list sizes and the number of hypergeometric evaluations.
- `pnhyper()` and `logdcDhyper()` instantiate new `HypergeometricDistribution` objects repeatedly instead of sharing one instance.
- `dnhyper()` has no internal guard for `ncp == 0`; the zero and infinity cases are safe only because `mnhyper()` intercepts them first.
- `UnirootZeroIn.zeroinC()` has no maximum-iteration cap. Its safety depends on `mle()` feeding it a well-behaved, bracketed monotone function.
- Confidence intervals are absent. That absence is part of current Java behavior and should be preserved for parity.