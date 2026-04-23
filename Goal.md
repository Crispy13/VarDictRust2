# Project Goal
This project is to port VarDictJava to Rust. VDR(VarDictRust) should output the same results as VDJ(VarDictJAVA), byte-identical.
Also, VDR should be faster and more resource efficient than VDJ.

Maximum Goal is 3X faster than VDJ, and similar or less memory usage than VDJ.

## Phase
### Phase 1: Porting 
In this phase, we will port the Java Code script by script (as best as possible). We only focus on logic matching and output matching. Ignore all performance things and idiomatic Rust things. Just Faithfully port the code so that it produces the same output as VDJ and has the same logic. But because of language differences, we may need to make some adjustments, but faithful porting is the first priority.

I am planning at least 5 bams to test the output parity.
**1. Making codebase documents:** Agents have limited context window, so they can't hold the whole codebase in memory. To solve this problem, we make codebase documents in detail. Maybe you can use "codebase-doc-manage" skill. 
**2. Porting code:**
    Port code. We port one by one, not all at once. And we can go on next steps only if output parity is achieved.
    - TDD approach: Make failed tests first with VDJ output, then port code to make tests pass. 

### Phase 2: Refactoring and Optimization
In this phase, we will refactor the code to make it more idiomatic Rust and optimize. Everytime we modify the code, we should check the output parity with VDJ. 


We hope this will be cost-effective improved version for the bioinformatics community.

## Parity scope

The formal claim boundary for "byte-identical output" is defined in
[docs/parity-scope.md](docs/parity-scope.md). Out-of-scope items (multi-threaded
execution, AmpliconMode, SplicingMode, CRAM input) are enumerated there and linked to
their respective handoff docs.