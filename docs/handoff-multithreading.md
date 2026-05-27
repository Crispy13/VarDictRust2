# Handoff: Multi-Threaded Execution (Rust `parallel()` Port)

**Status:** Partial acceptance. `parallel()` is implemented and determinism-tested; the
fixed `CM-TH4` preset now exercises bounded SimpleMode multi-thread coverage in the
config matrix.  
**Blocking:** Full multi-thread parity still requires broader sweep acceptance beyond
`CM-TH4`.  
**Depends on:** Single-threaded SimpleMode + SomaticMode parity complete (current state as of commit `3bd1115`).

---

## 1. What's in Java

[VarDictJava/src/main/java/com/astrazeneca/vardict/VarDictLauncher.java#L58-L65](VarDictJava/src/main/java/com/astrazeneca/vardict/VarDictLauncher.java#L58-L65):

```java
if (instance().conf.threads == 1)
    mode.notParallel();
else
    mode.parallel();
```

Every `AbstractMode` subclass (`SimpleMode`, `SomaticMode`, `AmpliconMode`, `SplicingMode`) implements **both** `notParallel()` and `parallel()`. The latter delegates to `createParallelMode().process()`.

### Core pattern ([AbstractMode.java#L102-L142](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/AbstractMode.java#L102))

```java
protected static abstract class AbstractParallelMode {
    static final int CAPACITY = 10;
    final ExecutorService executor = Executors.newFixedThreadPool(conf.threads);
    final BlockingQueue<Future<OutputStream>> toPrint = new LinkedBlockingQueue<>(CAPACITY);

    void process() {
        executor.submit(() -> produceTasks());  // producer thread
        while (true) {                           // consumer loop
            Future<OutputStream> wrk = toPrint.take();
            if (wrk == LAST_SIGNAL_FUTURE) break;
            VariantPrinter.createPrinter(printerTypeOut).print(wrk.get());
        }
        executor.shutdown();
    }
    abstract void produceTasks();
}
```

### Deterministic-output invariants

1. **Submission-order printing.** Consumer takes `Future`s from the BlockingQueue in the order the producer submitted them. Output is identical to single-threaded regardless of `-th` value.
2. **Bounded queue** (`CAPACITY=10`) provides backpressure — producer blocks when consumer lags.
3. **Per-region task granularity.** Each `Future<OutputStream>` contains the serialized output for one region. No cross-region data dependency.
4. **Sentinel termination.** `LAST_SIGNAL_FUTURE = CompletableFuture.completedFuture(null)` signals end-of-stream to the consumer.
5. **GlobalReadOnlyScope immutability.** `GlobalReadOnlyScope.instance().conf` is read-only after `start()`; no cross-thread mutation.

Per-mode `produceTasks()` implementations vary:
- `SimpleMode.createParallelMode()` at [SimpleMode.java#L54](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/SimpleMode.java#L54) — segment-then-region scatter.
- `SomaticMode.createParallelMode()` at [SomaticMode.java#L58](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/SomaticMode.java#L58) — pair submission per region, tumor + normal processed together.
- `AmpliconMode.createParallelMode()` at [AmpliconMode.java#L79](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/AmpliconMode.java#L79) — out of scope (AmpliconMode is deferred).
- `SplicingMode.createParallelMode()` at [SplicingMode.java#L67](VarDictJava/src/main/java/com/astrazeneca/vardict/modes/SplicingMode.java#L67) — out of scope.

---

## 2. What's in Rust

`parallel()` support is now present for the ported threaded modes, and the CLI/test
harnesses can exercise it.

- [src/bin/vardict_rs.rs](src/bin/vardict_rs.rs) accepts both `--th` and Java-style
  `-th` and dispatches SimpleMode to `parallel()` when `threads > 1`.
- [src/modes.rs](src/modes.rs) exposes `parallel()` for the ported modes used by the
  determinism harness.
- The parity harness now includes `CM-TH4` in
  [scripts/config_presets.tsv](scripts/config_presets.tsv) and applies a bounded
  thread budget in config/sweep harnesses so `parallel()` coverage is real without
  unconstrained oversubscription.
- The singleton `GlobalReadOnlyScope` in [src/scope.rs](src/scope.rs) remains
  `OnceLock`-based and thread-safe for reads.

---

## 3. Port Design Sketch

### 3.1 Threading primitive

**Recommended: [rayon](https://crates.io/crates/rayon) for the thread pool + [crossbeam-channel](https://crates.io/crates/crossbeam-channel) for the bounded FIFO.**

Rationale:
- Rayon already used elsewhere in the sweep harness (`RAYON_NUM_THREADS` env var in CI).
- `crossbeam-channel::bounded(10)` replicates Java's `LinkedBlockingQueue` semantics exactly.
- Avoid `tokio` — async runtime is overkill here; the workload is CPU-bound per region.

Alternative: `std::sync::mpsc::sync_channel(10)` is standard-library-only but less ergonomic for shutdown signaling. Pick crossbeam.

### 3.2 API shape

```text
pub trait ParallelMode {
    fn parallel(&self, thread_count: usize);
}

impl ParallelMode for SimpleMode { ... }
impl ParallelMode for SomaticMode { ... }
```

### 3.3 Pseudo-implementation

```text
fn parallel(&self, threads: usize) {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build()?;
    let (tx, rx) = crossbeam_channel::bounded::<oneshot::Receiver<Vec<u8>>>(10);

    // Producer: spawned on the pool, iterates segments/regions, submits tasks
    pool.spawn(move || {
        for region in self.regions() {
            let (task_tx, task_rx) = oneshot::channel();
            tx.send(task_rx).unwrap();
            pool.spawn(move || {
                let buf = process_region(region);  // same as not_parallel pipeline
                task_tx.send(buf).unwrap();
            });
        }
        drop(tx);  // closes channel = sentinel
    });

    // Consumer (caller thread): take futures in order, write to stdout
    let mut stdout = std::io::stdout().lock();
    for task_rx in rx {
        let buf = task_rx.recv().unwrap();
        stdout.write_all(&buf).unwrap();
    }
}
```

### 3.4 Determinism contract

**The output of `parallel(N)` MUST be byte-identical to `not_parallel()` for any N ≥ 1.**

This is non-negotiable for parity. Test harness must assert this directly (see §5).

---

## 4. Tricky Spots

1. **Per-region buffering.** Each task must write to an in-memory buffer (`Vec<u8>`), not to stdout directly. Streaming directly from worker threads would interleave output and break byte-identity. Java does this via `Future<OutputStream>` where `OutputStream` is actually `ByteArrayOutputStream`.

2. **Panic propagation.** Java's `executor.submit().get()` propagates exceptions from worker to consumer. Rust's `oneshot::Receiver` returns `Err` on sender drop but panics inside a task abort the whole process by default. Decide: `catch_unwind` per-task and surface via a `Result<Vec<u8>, PanicPayload>`, or let the process die. Java dies, so dying is the parity-safe choice.

3. **GlobalReadOnlyScope initialization.** Java populates `GlobalReadOnlyScope` before spawning threads. Rust's singleton must be initialized before `parallel()` too. If the test harness runs `parallel()` as part of a test, the singleton is already set by the caller. Verify this in tests.

4. **Output flushing and encoding.** `variantPrinter.print(wrk.get())` in Java writes a `ByteArrayOutputStream` that is UTF-8 byte-for-byte. Rust must preserve this — do not re-encode through `String`; use `Vec<u8>` and `write_all`.

5. **SomaticMode paired-BAM sharing.** `SomaticMode` opens two BAM files (tumor + normal) per-region. Rust must ensure both readers are `Send` or per-thread-local. `rust-htslib`'s `IndexedReader` is not `Send`; wrap in `thread_local!` or open fresh per task.

6. **Thread-count semantics.** Java `-th 1` takes the `notParallel()` branch; `-th 0` defaults to available cores. Rust must match. Record in a test: `-th 1` sweep result MUST equal no-flag sweep result.

7. **Reference resource sharing.** `ReferenceResource` is shared read-only in Java. Rust's equivalent (`src/reference.rs`) uses `Arc` — already safe, but verify no interior mutability.

---

## 5. Acceptance Criteria

The port is complete when all of the following are byte-identical:

1. `not_parallel()` output == Java `-th 1` output ✅ (already asserted, current state).
2. `parallel(1)` output == `not_parallel()` output.
3. `parallel(4)` output == `not_parallel()` output.
4. `parallel(8)` output == Java `-th 8` output.
5. `parallel(N)` output == `parallel(M)` output for all N, M ∈ {1, 2, 4, 8}.

These must hold for **both** SimpleMode and SomaticMode on **all** existing sweep tags (`hg002_agilent_v5`, `na12878_chrom20_exome`, `na12878_low_coverage`, `wes_il_pair`).

### Test scaffold to add

- `tests/parity_parallel_determinism.rs` — asserts parallel(N) == not_parallel for N ∈ {1, 4, 8} across all 4 tags × default config. Use a single tile to keep runtime small.
- `CM-TH4` (`-th 4`) is now present in `scripts/config_presets.tsv` and exercises the
  real bounded `parallel()` path in the config matrix. Broader thread-count acceptance
  remains future work.

---

## 6. Related Work / Links

- Committee report: [/memories/session/committee/consolidated-report.md](/memories/session/committee/consolidated-report.md) — Finding 10 (GPT unique).
- Original flag wiring: [src/bin/vardict_rs.rs](src/bin/vardict_rs.rs) accepts `-th` but ignores its value.
- Rust scope singleton: [src/scope.rs](src/scope.rs) — already thread-safe.
- Existing rayon usage: `scripts/sweep_fixtures_parallel.py` (Python-level parallelism, unrelated).
- Reference BAM reader: [src/mods/sam_file_parser.rs](src/mods/sam_file_parser.rs) — check `Send` bounds.

---

## 7. Estimated Effort

- Thread-pool + task queue scaffolding: small.
- Per-mode `parallel()` implementations (Simple + Somatic only): medium — have to refactor `not_parallel()` loops to yield per-region buffers instead of printing inline.
- Determinism test scaffold: small.
- Debugging non-deterministic output (HashMap iteration order, etc.): unknown; potentially painful. Rust `HashMap` is already non-deterministic per-instance; per-parity rules, must use `BTreeMap` or seeded hashers where Java uses `LinkedHashMap`.

**Risk:** Unknown how many code paths currently rely on `HashMap` iteration order that happens to match Java by coincidence under single-threaded execution; parallel execution may expose these. Audit all `HashMap` usage in `src/` under `rg 'HashMap<'` before starting.

---

## 8. Not This Subproject

- AmpliconMode parallel — out of scope; AmpliconMode itself is deferred.
- SplicingMode parallel — out of scope; SplicingMode itself is deferred.
- Distributed (multi-process) execution — not in Java either.
- GPU offload — not relevant.
