// parity_sweep_suite: all sweep tests live here.
// Uses GlobalReadOnlyScope::init()/clear() (process-global singleton).
// MUST be invoked with `--test-threads=1`.
#[path = "common/mod.rs"]
mod common;

#[path = "parity_sweep_suite/cigar_modifier_sweep.rs"]
mod cigar_modifier_sweep;

#[path = "parity_sweep_suite/cigar_parser_sweep.rs"]
mod cigar_parser_sweep;

#[path = "parity_sweep_suite/realigner_sweep.rs"]
mod realigner_sweep;

#[path = "parity_sweep_suite/sam_file_parser_sweep.rs"]
mod sam_file_parser_sweep;

#[path = "parity_sweep_suite/sv_processor_sweep.rs"]
mod sv_processor_sweep;

#[path = "parity_sweep_suite/tovars_sweep.rs"]
mod tovars_sweep;