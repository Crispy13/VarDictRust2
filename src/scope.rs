use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use once_cell::sync::Lazy;

use crate::config::{Configuration, PrinterType};
use crate::data::Region;
use crate::reference::{Reference, ReferenceResource};

/// Ported from: VariantPrinter.java placeholder for Scope.java:L14-L24 consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantPrinter {
    Out,
    Err,
}

impl Default for VariantPrinter {
    fn default() -> Self {
        Self::Out
    }
}

impl From<PrinterType> for VariantPrinter {
    fn from(value: PrinterType) -> Self {
        match value {
            PrinterType::Out => Self::Out,
            PrinterType::Err => Self::Err,
        }
    }
}

/// Ported from: AbstractMode.java placeholder for GlobalReadOnlyScope.java:L29-L41.
pub trait AbstractMode: Send + Sync {}

pub type SharedMode = Arc<dyn AbstractMode + Send + Sync>;

/// Ported from: Scope.java:L12-L36
/// Generic pipeline scope carrying common context plus stage-specific data.
#[derive(Clone, Debug)]
pub struct Scope<T> {
    pub bam: String,
    pub region: Region,
    pub region_ref: Arc<Reference>,
    pub reference_resource: Arc<ReferenceResource>,
    pub max_read_length: i32,
    pub splice: Arc<HashSet<String>>,
    pub out: Arc<VariantPrinter>,
    pub data: T,
}

impl<T> Scope<T> {
    /// Ported from: Scope.Scope(String, Region, Reference, ReferenceResource, int, Set<String>, VariantPrinter, T)
    /// Java source: Scope.java:L24-L31
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bam: impl Into<String>,
        region: Region,
        region_ref: Arc<Reference>,
        reference_resource: Arc<ReferenceResource>,
        max_read_length: i32,
        splice: HashSet<String>,
        out: VariantPrinter,
        data: T,
    ) -> Self {
        Self {
            bam: bam.into(),
            region,
            region_ref,
            reference_resource,
            max_read_length,
            splice: Arc::new(splice),
            out: Arc::new(out),
            data,
        }
    }

    /// Ported from: Scope.Scope(Scope<?>, T)
    /// Java source: Scope.java:L33-L36
    pub fn with_data<U>(self, data: U) -> Scope<U> {
        Scope {
            bam: self.bam,
            region: self.region,
            region_ref: self.region_ref,
            reference_resource: self.reference_resource,
            max_read_length: self.max_read_length,
            splice: self.splice,
            out: self.out,
            data,
        }
    }
}

/// Ported from: GlobalReadOnlyScope.java:L11-L69
/// Write-once global configuration and metadata shared across the pipeline.
#[derive(Clone, Debug)]
pub struct GlobalReadOnlyScope {
    pub conf: Configuration,
    pub chr_lengths: HashMap<String, i32>,
    pub sample: String,
    pub samplem: Option<String>,
    pub amplicon_based_calling: Option<String>,
    pub printer_type_out: PrinterType,
    pub adaptor_forward: HashMap<String, i32>,
    pub adaptor_reverse: HashMap<String, i32>,
}

static GLOBAL_READ_ONLY_SCOPE: Lazy<RwLock<Option<GlobalReadOnlyScope>>> =
    Lazy::new(|| RwLock::new(None));
static GLOBAL_MODE: Lazy<RwLock<Option<SharedMode>>> = Lazy::new(|| RwLock::new(None));

fn read_global_scope() -> Option<GlobalReadOnlyScope> {
    match GLOBAL_READ_ONLY_SCOPE.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn write_global_scope(scope: Option<GlobalReadOnlyScope>) {
    match GLOBAL_READ_ONLY_SCOPE.write() {
        Ok(mut guard) => *guard = scope,
        Err(poisoned) => *poisoned.into_inner() = scope,
    }
}

fn read_global_mode() -> Option<SharedMode> {
    match GLOBAL_MODE.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn write_global_mode(mode: Option<SharedMode>) {
    match GLOBAL_MODE.write() {
        Ok(mut guard) => *guard = mode,
        Err(poisoned) => *poisoned.into_inner() = mode,
    }
}

impl GlobalReadOnlyScope {
    /// Ported from: GlobalReadOnlyScope.GlobalReadOnlyScope(...)
    /// Java source: GlobalReadOnlyScope.java:L54-L69
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conf: Configuration,
        chr_lengths: HashMap<String, i32>,
        sample: impl Into<String>,
        samplem: Option<String>,
        amplicon_based_calling: Option<String>,
        adaptor_forward: HashMap<String, i32>,
        adaptor_reverse: HashMap<String, i32>,
    ) -> Self {
        let printer_type_out = conf.printer_type;
        Self {
            conf,
            chr_lengths,
            sample: sample.into(),
            samplem,
            amplicon_based_calling,
            printer_type_out,
            adaptor_forward,
            adaptor_reverse,
        }
    }

    /// Ported from: GlobalReadOnlyScope.instance()
    /// Java source: GlobalReadOnlyScope.java:L15-L17
    pub fn instance() -> GlobalReadOnlyScope {
        read_global_scope().expect("GlobalReadOnlyScope was not initialized.")
    }

    /// Ported from: GlobalReadOnlyScope.init(...)
    /// Java source: GlobalReadOnlyScope.java:L19-L27
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        conf: Configuration,
        chr_lengths: HashMap<String, i32>,
        sample: impl Into<String>,
        samplem: Option<String>,
        amplicon_based_calling: Option<String>,
        adaptor_forward: HashMap<String, i32>,
        adaptor_reverse: HashMap<String, i32>,
    ) {
        if read_global_scope().is_some() {
            panic!("GlobalReadOnlyScope was already initialized. Must be initialized only once.");
        }

        write_global_scope(Some(Self::new(
            conf,
            chr_lengths,
            sample,
            samplem,
            amplicon_based_calling,
            adaptor_forward,
            adaptor_reverse,
        )));
    }

    /// Ported from: GlobalReadOnlyScope.getMode()
    /// Java source: GlobalReadOnlyScope.java:L31-L33
    pub fn get_mode() -> Option<SharedMode> {
        read_global_mode()
    }

    /// Ported from: GlobalReadOnlyScope.setMode(AbstractMode)
    /// Java source: GlobalReadOnlyScope.java:L35-L40
    pub fn set_mode(run_mode: SharedMode) {
        if read_global_mode().is_some() {
            panic!(
				"Mode was already initialized for GlobalReadOnlyScope. Must be initialized only once."
			);
        }

        write_global_mode(Some(run_mode));
    }

    /// Ported from: GlobalReadOnlyScope.clear()
    /// Java source: GlobalReadOnlyScope.java:L45-L50
    pub fn clear() {
        write_global_scope(None);
        write_global_mode(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyMode;

    impl AbstractMode for DummyMode {}

    #[test]
    fn scope_with_data_preserves_shared_context() {
        let region_ref = Arc::new(Reference::default());
        let reference_resource = Arc::new(ReferenceResource::default());
        let mut splice = HashSet::new();
        splice.insert(String::from("10-12"));
        let scope = Scope::new(
            "sample.bam",
            Region::new("chr1", 10, 20, "GENE1"),
            region_ref.clone(),
            reference_resource.clone(),
            151,
            splice,
            VariantPrinter::Err,
            7_i32,
        );

        let next_scope = scope.with_data(String::from("next"));

        assert_eq!(next_scope.bam, "sample.bam");
        assert_eq!(next_scope.region.print_region(), "chr1:10-20");
        assert!(Arc::ptr_eq(&next_scope.region_ref, &region_ref));
        assert!(Arc::ptr_eq(
            &next_scope.reference_resource,
            &reference_resource
        ));
        assert_eq!(next_scope.max_read_length, 151);
        assert!(next_scope.splice.contains("10-12"));
        assert_eq!(*next_scope.out, VariantPrinter::Err);
        assert_eq!(next_scope.data, "next");
    }

    #[test]
    fn global_read_only_scope_lifecycle_supports_clear_and_reinit() {
        GlobalReadOnlyScope::clear();

        let mut chr_lengths = HashMap::new();
        chr_lengths.insert(String::from("chr1"), 249_250_621);
        let mut adaptor_forward = HashMap::new();
        adaptor_forward.insert(String::from("AAAAAA"), 1);
        let mut adaptor_reverse = HashMap::new();
        adaptor_reverse.insert(String::from("TTTTTT"), 1);

        GlobalReadOnlyScope::init(
            Configuration::default(),
            chr_lengths.clone(),
            "tumor",
            Some(String::from("normal")),
            Some(String::from("10:0.95")),
            adaptor_forward.clone(),
            adaptor_reverse.clone(),
        );

        let instance = GlobalReadOnlyScope::instance();
        assert_eq!(instance.sample, "tumor");
        assert_eq!(instance.samplem.as_deref(), Some("normal"));
        assert_eq!(instance.amplicon_based_calling.as_deref(), Some("10:0.95"));
        assert_eq!(instance.chr_lengths, chr_lengths);
        assert_eq!(instance.adaptor_forward, adaptor_forward);
        assert_eq!(instance.adaptor_reverse, adaptor_reverse);
        assert_eq!(instance.printer_type_out, PrinterType::Out);

        GlobalReadOnlyScope::set_mode(Arc::new(DummyMode));
        assert!(GlobalReadOnlyScope::get_mode().is_some());

        GlobalReadOnlyScope::clear();
        assert!(GlobalReadOnlyScope::get_mode().is_none());

        GlobalReadOnlyScope::init(
            Configuration::default(),
            HashMap::new(),
            "sample",
            None,
            None,
            HashMap::new(),
            HashMap::new(),
        );

        assert_eq!(GlobalReadOnlyScope::instance().sample, "sample");
        GlobalReadOnlyScope::clear();
    }
}
