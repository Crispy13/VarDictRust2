# JSONL Format Contract

Authoritative spec for the module parity test system. Both Java writers and Rust serde output must produce bytes matching this contract.

## File Structure

```
Line 1: {"__meta":{"module":"<MODULE>","region":"<CHR>:<START>-<END>"}}
Line 2: <DATA_JSON>
```

- Exactly 2 lines per file.
- Line 1 is metadata — skipped during comparison.
- Line 2 is the full serialized scope data — this is what's compared byte-for-byte.

## Type Mappings

| Java Type | JSONL Representation | Rust Type |
|-----------|---------------------|-----------|
| `int` | number (no decimal) | `i32` |
| `Integer` (null) | `null` | `Option<i32>` |
| `double` | number via `Double.toString()` | `f64` |
| `boolean` | `true`/`false` | `bool` |
| `String` (null) | `null` | `Option<String>` |
| `String` | `"escaped"` | `String` |
| `HashMap<Integer, T>` | `[[key, value], ...]` sorted by key | `HashMap<i32, T>` + `#[serde(serialize_with)]` |
| `LinkedHashMap<K, V>` | `[[key, value], ...]` insertion order | `IndexMap<K, V>` |
| `TreeMap<K, V>` | `[[key, value], ...]` sorted by key | `BTreeMap<K, V>` |
| `VariationMap<String, Variation>` | `{"entries": [[k,v],...], "sv": ...}` | `VariationMap` struct |
| `List<T>` | `[...]` | `Vec<T>` |
| `Set<String>` | `[...]` sorted alphabetically | `BTreeSet<String>` or sorted `HashSet` |
| `int[]` | `[n1, n2, ...]` | `Vec<i32>` or `[i32; N]` |

## Float Format

Both Java `Double.toString(x)` and Rust `serde_json` (Ryu) produce the shortest round-trip-safe decimal representation. They should be identical for all IEEE 754 doubles encountered in practice.

**Exception:** Values formatted through Java's `DecimalFormat` (e.g. output fields) may use HALF_UP rounding. These are NOT in the module JSONL output — they only appear in final TSV output. Module-level JSONL captures raw doubles.

## Field Naming

JSON keys match Java field names exactly. Rust structs use `#[serde(rename = "javaFieldName")]` when snake_case differs from camelCase:

```rust
#[derive(Serialize)]
pub struct Variation {
    #[serde(rename = "varsCount")]
    pub vars_count: i32,
    #[serde(rename = "varsCountOnForward")]
    pub vars_count_on_forward: i32,
}
```

## Map Serialization

### HashMap (unordered) → sorted `[[key, value], ...]`

All `HashMap` keys are sorted before serialization to ensure deterministic output. Java side sorts in `writeSortedIntMap()`. Rust side uses a custom serializer:

```rust
#[serde(serialize_with = "serialize_sorted_int_map")]
pub non_insertion_variants: HashMap<i32, VariationMap>,
```

### VariationMap (insertion-ordered + SV side-field)

Always serialized as a wrapper object, even when `sv` is null:

```json
{"entries": [["A", {...}], ["T", {...}]], "sv": null}
```

This avoids conditional parsing on the Rust side.

## Null Conventions

- **All fields always present.** Null → `null`, 0 → `0`, empty string → `""`. No field omission.
- **Null collections → `null`.** Not `[]`.

## Java Writer Pattern

Java writers use `StringBuilder` with no external JSON library. The `parity/` package has:

| File | Purpose |
|------|---------|
| `JsonlConfig.java` | Env-var reader (`VARDICT_PARITY_*`) |
| `JsonlWriter.java` | File creation, meta envelope |
| `VariationJsonl.java` | Core type serializers (Variation, Sclip, Mate, SV) |
| `CigarParserJsonl.java` | CigarParser scope data writer |
| `RealignerJsonl.java` | Realigner scope data writer |
| `SVProcessorJsonl.java` | SVProcessor scope data writer |
| `ToVarsBuilderJsonl.java` | ToVars scope data writer |

Call-site injection is a one-liner guard:
```java
String jsonlDir = JsonlConfig.getOutputDir("CIGAR_PARSER");
if (jsonlDir != null) {
    CigarParserJsonl.write(jsonlDir, region, variationData);
}
```

## Env Vars for Fixture Generation

| Module | Env Var |
|--------|---------|
| CigarParser | `VARDICT_PARITY_CIGAR_PARSER` |
| Realigner | `VARDICT_PARITY_REALIGNER` |
| SVProcessor | `VARDICT_PARITY_SV_PROCESSOR` |
| ToVars | `VARDICT_PARITY_TOVARS` |

Set to the output directory path to enable JSONL snapshot writing.
