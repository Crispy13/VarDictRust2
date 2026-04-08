---
description: 'Parity-specific Rust rules for VarDict-rs faithful port. Type mappings, float formatting, collection ordering, integer overflow, null handling, traceability comments.'
applyTo: '**/*.rs'
---

# Rust Parity Rules for VarDict-rs

Use these rules when translating Java VarDict logic into Rust. The bar is behavior parity first,
idiomatic cleanup later. If a choice changes emitted values, ordering, null propagation, or string
formatting, prefer the Java-compatible behavior.

## 1. Type Mapping Reference

Type selection is parity-critical. Do not "improve" a Java type unless you can prove the change is
output-neutral.

| Java | Rust | Parity note |
|------|------|-------------|
| `int` | `i32` | Same width and signed wraparound model |
| `long` | `i64` | Same width and signed range |
| `double` | `f64` | Use `f64`, never downgrade to `f32` |
| `float` | `f32` only if Java source actually uses `float` | Do not guess; many ports should stay `f64` |
| `char` | `u8` for genomic ASCII, `char` only when Unicode semantics matter | Prefer `u8` for indexing and base manipulation |
| `boolean` | `bool` | Direct mapping |
| `String` | `String` | Use owned strings for Java-owned data, not borrowed `&str` fields |
| `StringBuilder` | `String` plus `push` / `push_str` | Preserve append order exactly |
| `Integer` | `Option<i32>` | Boxed primitives are nullable |
| `Long` | `Option<i64>` | Boxed primitives are nullable |
| `Double` | `Option<f64>` | Boxed primitives are nullable |
| `null` reference | `None` | Every Java null path must become an explicit `Option` path |
| `List<T>` / `ArrayList<T>` | `Vec<T>` | Preserve element order |
| `List<Integer>` with nulls | `Vec<Option<i32>>` | Nullability inside the collection still matters |
| `ArrayDeque<T>` | `VecDeque<T>` or `Vec<T>` | Match queue or stack semantics intentionally |
| `HashMap<K, V>` | `HashMap<K, V>` only when order never affects output | Treat every `HashMap` as suspicious in parity code |
| `LinkedHashMap<K, V>` | `IndexMap<K, V>` | Critical: insertion order must be preserved |
| `TreeMap<K, V>` | `BTreeMap<K, V>` | Sorted iteration order must remain sorted |
| `HashSet<T>` | `HashSet<T>` only when order never affects output | Never emit directly if order matters |
| `LinkedHashSet<T>` | `IndexSet<T>` | Preserve insertion order |
| `Integer.MAX_VALUE` / `Long.MAX_VALUE` | `i32::MAX` / `i64::MAX` | Use Rust constants directly |

Working rules:

- If Java used `LinkedHashMap` or `LinkedHashSet`, default to `IndexMap` or `IndexSet` immediately.
- If the Java value could be null, model that as `Option<T>` at the boundary, not after the value has
  already flowed through the port.
- For function parameters, borrowing is still fine when it does not change data modeling. For stored
  fields and parity-sensitive state, mirror Java ownership first.

## 2. Float Formatting

Formatting is a common parity failure source. Do not use raw Rust formatting for Java-originated
decimal output unless you have proven it matches the Java call site.

Primary rule:

- Use the project's parity formatting helper for Java-style decimal output.
- The committee guidance refers to this as `java_format_double()`.
- Existing repo utilities may still route some call sites through helpers in `src/utils.rs`, such as
  `round_half_even()` or `get_rounded_value_to_print()`. Keep the Java behavior of the original call
  site; do not replace it with `format!`.

Prefer:

```rust
let freq = java_format_double(value, 4);
let nm = java_format_double(nm_value, 1);
```

Avoid:

```rust
let freq = format!("{:.4}", value);
let nm = format!("{}", nm_value);
```

Additional rules:

- For parity fixtures carrying raw doubles, prefer default `serde_json` serialization rather than
  pre-formatting numeric values into strings.
- Preserve Java zero handling and trailing-zero behavior. Some call sites emit `"0"` for exact zero,
  while others preserve a fixed pattern such as `"0.0"` or `"0.0000"`.
- When a module doc or Java source shows `DecimalFormat`, port that call through the parity helper,
  not an ad hoc formatter.

Checklist for each float field:

- Is this value emitted as a string or rounded display field?
- Does Java preserve or strip trailing zeros here?
- Does Java special-case zero, positive-only values, or whole numbers?
- Are you using the shared parity helper instead of a new formatting expression?

## 3. Integer Overflow

Java integer arithmetic wraps silently. Rust debug builds panic on overflow. If the original Java code
could overflow and relied on normal `int` or `long` arithmetic, use explicit wrapping operations.

Use wrapping operations for ported Java arithmetic:

```rust
let total = count1.wrapping_add(count2);
let product = value.wrapping_mul(scale);
let mask = bits.wrapping_shl(shift);
```

Use wrapping semantics when:

- Porting accumulation loops, counters, hash-like state, or bit-manipulation logic from Java.
- The Java code performed arithmetic without overflow guards.
- Matching debug and release behavior matters for parity reproduction.

Do not use wrapping semantics when:

- The arithmetic represents an actual invalid state that should stop the computation.
- You are validating user input, region bounds, or lengths where overflow would indicate a bug.
- The operation is floating-point; use `f64` semantics instead.

If you choose `checked_*`, `saturating_*`, or plain `+`, be prepared to justify why Java parity does
not require wraparound at that site.

## 4. Collection Ordering

Iteration order is one of the most common hidden parity regressions.

Rules:

- `LinkedHashMap` in Java means `IndexMap` in Rust.
- `LinkedHashSet` in Java means `IndexSet` in Rust.
- `TreeMap` in Java means `BTreeMap` in Rust.
- `HashMap` or `HashSet` are acceptable only when order is provably irrelevant to output,
  downstream sorting, or serialized fixtures.

Example:

```rust
use indexmap::IndexMap;

let mut counts: IndexMap<char, i32> = IndexMap::new();
counts.insert('A', 3);
counts.insert('T', 1);
```

Nested structures must preserve order at every level:

```rust
use indexmap::IndexMap;

let mut outer: IndexMap<String, IndexMap<String, i32>> = IndexMap::new();
```

Do not mix an ordered outer map with an unordered inner map if the inner map is ever iterated for
output, comparison, or fixture generation.

Verification checklist:

- Does the Java source use `LinkedHashMap` or `LinkedHashSet`?
- Does iteration order feed JSON, TSV, debug output, or test fixtures?
- Does a nested map or set also need stable ordering?
- Are you sorting later? If so, is the pre-sort accumulation order still observable anywhere?

## 5. Null Handling

Java null semantics should become explicit Rust `Option` semantics at the first boundary where null is
possible.

Common mappings:

- Nullable boxed primitive: `Option<i32>`, `Option<i64>`, `Option<f64>`
- Nullable object reference: `Option<T>`
- Nullable collection: `Option<Vec<T>>`, `Option<IndexMap<K, V>>`, and so on
- `map.get(key)` style access: model the missing case explicitly rather than inventing a sentinel

Port patterns:

```rust
if let Some(field) = field {
    use_field(field);
}

match maybe_value {
    Some(value) => handle_value(value),
    None => handle_null_case(),
}
```

Serialization patterns:

```rust
#[derive(serde::Serialize)]
struct OutputRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_field: Option<String>,
}
```

Rules:

- Convert nullability early. Do not carry placeholder sentinels and retrofit `Option` later.
- Match Java propagation rules exactly: omitted field, explicit null, default value, or skipped branch.
- If Java distinguishes between an empty collection and null, Rust must keep that distinction.

## 6. Traceability Comments

Every non-trivial parity-sensitive function should carry a traceability comment that points back to
the Java source.

Required format:

```rust
/// Ported from: CigarParser.java:L142-L380
fn parse_cigar(...) {
    // ...
}
```

Guidance:

- Use the Java file and line span that actually corresponds to the Rust function.
- Keep the traceability comment immediately above the function or impl method.
- Add a short second doc line when the function purpose is not obvious.
- Trivial wrappers and tiny getters do not need forced documentation, but any logic that can affect
  parity output should have traceability.

## 7. Common Divergence Patterns

Check these first when parity fails.

| Symptom | Likely cause | First check |
|---------|--------------|-------------|
| Float value is numerically wrong | Integer division or wrong float type | Confirm `f64` math and casts happen at the same point as Java |
| Float text formatting differs | Raw `format!` or wrong parity helper | Replace ad hoc formatting with the shared parity helper |
| Map or set output order differs | `HashMap` / `HashSet` used where Java preserved order | Audit for `LinkedHashMap` / `LinkedHashSet` in Java |
| Nested JSON key order differs | Only outer map preserves order | Make inner maps `IndexMap` too |
| Null field appears or disappears | `Option` introduced too late or wrong serde rule | Re-check null propagation and serialization behavior |
| Extra or missing rows appear | Branch inversion, early return, or filter mismatch | Compare the first divergent branch with Java |
| Alleles, genotypes, or lists are in the wrong order | Sorting rule or insertion order mismatch | Match Java comparator and stable ordering exactly |
| Debug build panics but Java keeps running | Plain Rust arithmetic overflowed | Use `wrapping_*` where Java relied on silent wraparound |
| Output differs by one position or one element | Off-by-one translation from Java loop bounds | Re-check inclusive vs exclusive indices |
| Case-sensitive comparisons differ | Normalization changed during port | Match Java string and byte case handling exactly |

## 8. Validation Checklist

Before parity review, confirm all of the following:

- [ ] Every Java `LinkedHashMap` / `LinkedHashSet` became `IndexMap` / `IndexSet`
- [ ] Nested output-affecting collections also preserve order
- [ ] Nullable Java values became `Option<T>` at the boundary
- [ ] Float output uses the shared parity helper, not raw `format!`
- [ ] Overflow-sensitive Java arithmetic uses the correct Rust overflow behavior
- [ ] Traceability comments exist on non-trivial parity-sensitive functions
- [ ] `cargo fmt`, `cargo clippy`, and module validation are clean before handoff