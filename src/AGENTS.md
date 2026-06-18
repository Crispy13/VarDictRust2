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

# Rust Coding Conventions and Best Practices

Follow idiomatic Rust practices and community standards when writing Rust code. 

These instructions are based on [The Rust Book](https://doc.rust-lang.org/book/), [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), [RFC 430 naming conventions](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md), and the broader Rust community at [users.rust-lang.org](https://users.rust-lang.org).

## General Instructions

- Always prioritize readability, safety, and maintainability.
- Use strong typing and leverage Rust's ownership system for memory safety.
- Break down complex functions into smaller, more manageable functions.
- For algorithm-related code, include explanations of the approach used.
- Write code with good maintainability practices, including comments on why certain design decisions were made.
- Handle errors gracefully using `Result<T, E>` and provide meaningful error messages.
- For external dependencies, mention their usage and purpose in documentation.
- Use consistent naming conventions following [RFC 430](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md).
- Write idiomatic, safe, and efficient Rust code that follows the borrow checker's rules.
- Ensure code compiles without warnings.

## Patterns to Follow

- Use modules (`mod`) and public interfaces (`pub`) to encapsulate logic.
- Handle errors properly using `?`, `match`, or `if let`.
- Use `serde` for serialization and `thiserror` or `anyhow` for custom errors.
- Implement traits to abstract services or external dependencies.
- Structure async code using `async/await` and `tokio` or `async-std`.
- Prefer enums over flags and states for type safety.
- Use builders for complex object creation.
- Split binary and library code (`main.rs` vs `lib.rs`) for testability and reuse.
- Use `rayon` for data parallelism and CPU-bound tasks.
- Use iterators instead of index-based loops as they're often faster and safer.
- Use `&str` instead of `String` for function parameters when you don't need ownership.
- Prefer borrowing and zero-copy operations to avoid unnecessary allocations.

### Ownership, Borrowing, and Lifetimes

- Prefer borrowing (`&T`) over cloning unless ownership transfer is necessary.
- Use `&mut T` when you need to modify borrowed data.
- Explicitly annotate lifetimes when the compiler cannot infer them.
- Use `Rc<T>` for single-threaded reference counting and `Arc<T>` for thread-safe reference counting.
- Use `RefCell<T>` for interior mutability in single-threaded contexts and `Mutex<T>` or `RwLock<T>` for multi-threaded contexts.

## Patterns to Avoid

- Don't use `unwrap()` or `expect()` unless absolutely necessary—prefer proper error handling.
- Avoid panics in library code—return `Result` instead.
- Don't rely on global mutable state—use dependency injection or thread-safe containers.
- Avoid deeply nested logic—refactor with functions or combinators.
- Don't ignore warnings—treat them as errors during CI.
- Avoid `unsafe` unless required and fully documented.
- Don't overuse `clone()`, use borrowing instead of cloning unless ownership transfer is needed.
- Avoid premature `collect()`, keep iterators lazy until you actually need the collection.
- Avoid unnecessary allocations—prefer borrowing and zero-copy operations.

## Code Style and Formatting

- Follow the Rust Style Guide and use `rustfmt` for automatic formatting.
- Keep lines under 100 characters when possible.
- Place function and struct documentation immediately before the item using `///`.
- Use `cargo clippy` to catch common mistakes and enforce best practices.

## Error Handling

- Use `Result<T, E>` for recoverable errors and `panic!` only for unrecoverable errors.
- Prefer `?` operator over `unwrap()` or `expect()` for error propagation.
- Create custom error types using `thiserror` or implement `std::error::Error`.
- Use `Option<T>` for values that may or may not exist.
- Provide meaningful error messages and context.
- Error types should be meaningful and well-behaved (implement standard traits).
- Validate function arguments and return appropriate errors for invalid input.

## API Design Guidelines

### Common Traits Implementation
Eagerly implement common traits where appropriate:
- `Copy`, `Clone`, `Eq`, `PartialEq`, `Ord`, `PartialOrd`, `Hash`, `Debug`, `Display`, `Default`
- Use standard conversion traits: `From`, `AsRef`, `AsMut`
- Collections should implement `FromIterator` and `Extend`
- Note: `Send` and `Sync` are auto-implemented by the compiler when safe; avoid manual implementation unless using `unsafe` code

### Type Safety and Predictability
- Use newtypes to provide static distinctions
- Arguments should convey meaning through types; prefer specific types over generic `bool` parameters
- Use `Option<T>` appropriately for truly optional values
- Functions with a clear receiver should be methods
- Only smart pointers should implement `Deref` and `DerefMut`

### Future Proofing
- Use sealed traits to protect against downstream implementations
- Structs should have private fields
- Functions should validate their arguments
- All public types must implement `Debug`

## Testing and Documentation

- Write comprehensive unit tests using `#[cfg(test)]` modules and `#[test]` annotations.
- Use test modules alongside the code they test (`mod tests { ... }`).
- Write integration tests in `tests/` directory with descriptive filenames.
- Write clear and concise comments for each function, struct, enum, and complex logic.
- Ensure functions have descriptive names and include comprehensive documentation.
- Document all public APIs with rustdoc (`///` comments) following the [API Guidelines](https://rust-lang.github.io/api-guidelines/).
- Use `#[doc(hidden)]` to hide implementation details from public documentation.
- Document error conditions, panic scenarios, and safety considerations.
- Examples should use `?` operator, not `unwrap()` or deprecated `try!` macro.

## Project Organization

- Use semantic versioning in `Cargo.toml`.
- Include comprehensive metadata: `description`, `license`, `repository`, `keywords`, `categories`.
- Use feature flags for optional functionality.
- Organize code into modules using `mod.rs` or named files.
- Keep `main.rs` or `lib.rs` minimal - move logic to modules.

## Quality Checklist

Before publishing or reviewing Rust code, ensure:

### Core Requirements
- [ ] **Naming**: Follows RFC 430 naming conventions
- [ ] **Traits**: Implements `Debug`, `Clone`, `PartialEq` where appropriate
- [ ] **Error Handling**: Uses `Result<T, E>` and provides meaningful error types
- [ ] **Documentation**: All public items have rustdoc comments with examples
- [ ] **Testing**: Comprehensive test coverage including edge cases

### Safety and Quality
- [ ] **Safety**: No unnecessary `unsafe` code, proper error handling
- [ ] **Performance**: Efficient use of iterators, minimal allocations
- [ ] **API Design**: Functions are predictable, flexible, and type-safe
- [ ] **Future Proofing**: Private fields in structs, sealed traits where appropriate
- [ ] **Tooling**: Code passes `cargo fmt`, `cargo clippy`, and `cargo test -- --include-ignored`

## Project-Specific Preferences

1. Do not modify `Cargo.lock` directly. It should be modified by cargo commands.
2. Prefer `tracing` and `tracing-subscriber` for logging over `println!` or `eprintln!`.
	- Dev subscriber: include `.with_file(true).with_line_number(true).with_target(true)`.
	- Production subscriber: use `.with_file(false).with_line_number(false).with_target(false)`.
3. Prefer enums or generic code over dynamic dispatch (trait objects) for better performance and compile-time checks.
