use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex, RwLock};

use once_cell::sync::Lazy;

use crate::config::{Configuration, PrinterType};
use crate::data::Region;
use crate::reference::{Reference, ReferenceResource};

/// Ported from: VariantPrinter.java placeholder for Scope.java:L14-L24 consumers.
#[derive(Clone, Debug)]
pub enum VariantPrinter {
    Out,
    Err,
    Buffer(Arc<Mutex<String>>),
}

impl PartialEq for VariantPrinter {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Out, Self::Out) | (Self::Err, Self::Err) => true,
            (Self::Buffer(left), Self::Buffer(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for VariantPrinter {}

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

impl VariantPrinter {
    /// Ported from: VariantPrinter.print(OutputVariant)
    /// Java source: VariantPrinter.java:L17-L19
    pub fn print_line(&self, line: &str) {
        match self {
            Self::Out => {
                let stdout = std::io::stdout();
                let mut handle = stdout.lock();
                let _ = writeln!(handle, "{line}");
            }
            Self::Err => {
                let stderr = std::io::stderr();
                let mut handle = stderr.lock();
                let _ = writeln!(handle, "{line}");
            }
            Self::Buffer(buffer) => {
                let mut output = buffer.lock().unwrap_or_else(|error| error.into_inner());
                output.push_str(line);
                output.push('\n');
            }
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
    pub variant_printer: VariantPrinter,
    pub adaptor_forward: HashMap<String, i32>,
    pub adaptor_reverse: HashMap<String, i32>,
}

static GLOBAL_READ_ONLY_SCOPE: Lazy<RwLock<Option<GlobalReadOnlyScope>>> =
    Lazy::new(|| RwLock::new(None));
static GLOBAL_MODE: Lazy<RwLock<Option<SharedMode>>> = Lazy::new(|| RwLock::new(None));

thread_local! {
    static LOCAL_GROS: RefCell<Option<GlobalReadOnlyScope>> = RefCell::new(None);
    static LOCAL_MODE: RefCell<Option<SharedMode>> = RefCell::new(None);
}

const _: fn() = {
    fn check() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<GlobalReadOnlyScope>();
    }

    check
};

fn read_local_scope() -> Option<GlobalReadOnlyScope> {
    LOCAL_GROS.with(|scope| scope.borrow().clone())
}

fn write_local_scope(scope: Option<GlobalReadOnlyScope>) {
    LOCAL_GROS.with(|local_scope| *local_scope.borrow_mut() = scope);
}

fn read_local_mode() -> Option<SharedMode> {
    LOCAL_MODE.with(|mode| mode.borrow().clone())
}

fn write_local_mode(mode: Option<SharedMode>) {
    LOCAL_MODE.with(|local_mode| *local_mode.borrow_mut() = mode);
}

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
        let variant_printer = VariantPrinter::from(printer_type_out);
        Self {
            conf,
            chr_lengths,
            sample: sample.into(),
            samplem,
            amplicon_based_calling,
            printer_type_out,
            variant_printer,
            adaptor_forward,
            adaptor_reverse,
        }
    }

    pub fn try_instance() -> Option<GlobalReadOnlyScope> {
        read_local_scope().or_else(read_global_scope)
    }

    pub fn try_thread_local_instance() -> Option<GlobalReadOnlyScope> {
        read_local_scope()
    }

    pub fn with_thread_local_instance<R>(f: impl FnOnce(Option<&GlobalReadOnlyScope>) -> R) -> R {
        LOCAL_GROS.with(|scope| {
            let scope = scope.borrow();
            f(scope.as_ref())
        })
    }

    /// Ported from: GlobalReadOnlyScope.instance()
    /// Java source: GlobalReadOnlyScope.java:L15-L17
    pub fn instance() -> GlobalReadOnlyScope {
        Self::try_instance().expect("GlobalReadOnlyScope was not initialized.")
    }

    /// Borrow the active scope without cloning large read-only maps.
    pub fn with_instance<R>(f: impl FnOnce(&GlobalReadOnlyScope) -> R) -> R {
        if LOCAL_GROS.with(|scope| scope.borrow().is_some()) {
            return LOCAL_GROS.with(|scope| {
                let scope = scope.borrow();
                f(scope
                    .as_ref()
                    .expect("GlobalReadOnlyScope was not initialized."))
            });
        }

        match GLOBAL_READ_ONLY_SCOPE.read() {
            Ok(guard) => f(guard
                .as_ref()
                .expect("GlobalReadOnlyScope was not initialized.")),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                f(guard
                    .as_ref()
                    .expect("GlobalReadOnlyScope was not initialized."))
            }
        }
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

    /// Installs a thread-local scope override for the current thread only.
    #[allow(clippy::too_many_arguments)]
    pub fn init_thread_local(
        conf: Configuration,
        chr_lengths: HashMap<String, i32>,
        sample: impl Into<String>,
        samplem: Option<String>,
        amplicon_based_calling: Option<String>,
        adaptor_forward: HashMap<String, i32>,
        adaptor_reverse: HashMap<String, i32>,
    ) {
        if read_local_scope().is_some() {
            panic!(
                "Thread-local GlobalReadOnlyScope was already initialized. Must be initialized only once per thread."
            );
        }

        write_local_scope(Some(Self::new(
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
        read_local_mode().or_else(read_global_mode)
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

    /// Installs a thread-local mode override for the current thread only.
    pub fn set_mode_thread_local(run_mode: SharedMode) {
        if read_local_mode().is_some() {
            panic!(
                "Thread-local mode was already initialized for GlobalReadOnlyScope. Must be initialized only once per thread."
            );
        }

        write_local_mode(Some(run_mode));
    }

    pub fn set_variant_printer(printer: VariantPrinter) {
        let updated_local_scope = LOCAL_GROS.with(|local_scope| {
            let mut local_scope = local_scope.borrow_mut();
            if let Some(scope) = local_scope.as_mut() {
                scope.variant_printer = printer.clone();
                true
            } else {
                false
            }
        });

        if updated_local_scope {
            return;
        }

        match GLOBAL_READ_ONLY_SCOPE.write() {
            Ok(mut guard) => {
                let scope = guard
                    .as_mut()
                    .expect("GlobalReadOnlyScope must be initialized before setting printer");
                scope.variant_printer = printer;
            }
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                let scope = guard
                    .as_mut()
                    .expect("GlobalReadOnlyScope must be initialized before setting printer");
                scope.variant_printer = printer;
            }
        }
    }

    /// Ported from: GlobalReadOnlyScope.clear()
    /// Java source: GlobalReadOnlyScope.java:L45-L50
    pub fn clear() {
        write_global_scope(None);
        write_global_mode(None);
    }

    /// Clears thread-local scope and mode overrides for the current thread only.
    pub fn clear_thread_local() {
        write_local_scope(None);
        write_local_mode(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SCOPE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct DummyMode;

    struct AlternateDummyMode;

    impl AbstractMode for DummyMode {}

    impl AbstractMode for AlternateDummyMode {}

    struct ScopeStateGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopeStateGuard {
        fn new() -> Self {
            let lock = TEST_SCOPE_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            GlobalReadOnlyScope::clear_thread_local();
            Self { _lock: lock }
        }
    }

    impl Drop for ScopeStateGuard {
        fn drop(&mut self) {
            GlobalReadOnlyScope::clear_thread_local();
        }
    }

    fn install_global_scope(sample: &str) {
        write_global_scope(Some(GlobalReadOnlyScope::new(
            Configuration::default(),
            HashMap::new(),
            sample,
            None,
            None,
            HashMap::new(),
            HashMap::new(),
        )));
        write_global_mode(None);
    }

    fn init_thread_local_scope(sample: &str) {
        GlobalReadOnlyScope::init_thread_local(
            Configuration::default(),
            HashMap::new(),
            sample,
            None,
            None,
            HashMap::new(),
            HashMap::new(),
        );
    }

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
        let _guard = ScopeStateGuard::new();

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
        assert_eq!(instance.variant_printer, VariantPrinter::Out);

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
    }

    #[test]
    fn thread_local_scope_and_mode_override_global_fallback_on_current_thread() {
        let _guard = ScopeStateGuard::new();

        install_global_scope("global");
        let global_mode: SharedMode = Arc::new(DummyMode);
        GlobalReadOnlyScope::set_mode(global_mode);

        init_thread_local_scope("local");
        let local_mode: SharedMode = Arc::new(AlternateDummyMode);
        GlobalReadOnlyScope::set_mode_thread_local(local_mode.clone());

        assert_eq!(GlobalReadOnlyScope::instance().sample, "local");
        let active_mode = GlobalReadOnlyScope::get_mode().expect("local mode should be visible");
        assert!(Arc::ptr_eq(&active_mode, &local_mode));
    }

    #[test]
    fn set_variant_printer_updates_local_scope_when_override_exists() {
        let _guard = ScopeStateGuard::new();

        init_thread_local_scope("local");

        let output = Arc::new(Mutex::new(String::new()));
        let local_printer = VariantPrinter::Buffer(output);
        GlobalReadOnlyScope::set_variant_printer(local_printer.clone());

        assert_eq!(
            GlobalReadOnlyScope::instance().variant_printer,
            local_printer
        );
    }

    #[test]
    fn clear_thread_local_restores_global_fallback() {
        let _guard = ScopeStateGuard::new();

        install_global_scope("global");
        let global_mode: SharedMode = Arc::new(DummyMode);
        GlobalReadOnlyScope::set_mode(global_mode.clone());

        init_thread_local_scope("local");
        let local_mode: SharedMode = Arc::new(AlternateDummyMode);
        GlobalReadOnlyScope::set_mode_thread_local(local_mode);

        GlobalReadOnlyScope::clear_thread_local();

        assert_eq!(GlobalReadOnlyScope::instance().sample, "global");
        let active_mode = GlobalReadOnlyScope::get_mode().expect("global mode should be visible");
        assert!(Arc::ptr_eq(&active_mode, &global_mode));
    }

    #[test]
    fn fallback_without_thread_local_override_matches_existing_global_behavior() {
        let _guard = ScopeStateGuard::new();

        assert!(read_local_scope().is_none());
        assert!(read_local_mode().is_none());

        install_global_scope("global");
        let global_mode: SharedMode = Arc::new(DummyMode);
        GlobalReadOnlyScope::set_mode(global_mode.clone());

        assert_eq!(GlobalReadOnlyScope::instance().sample, "global");
        let active_mode = GlobalReadOnlyScope::get_mode().expect("global mode should be visible");
        assert!(Arc::ptr_eq(&active_mode, &global_mode));

        GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Err);
        assert_eq!(
            GlobalReadOnlyScope::instance().variant_printer,
            VariantPrinter::Err
        );
    }

    #[test]
    fn child_thread_does_not_inherit_parent_thread_local_scope() {
        let _guard = ScopeStateGuard::new();

        install_global_scope("global");
        let global_mode: SharedMode = Arc::new(DummyMode);
        GlobalReadOnlyScope::set_mode(global_mode.clone());

        init_thread_local_scope("local");
        let local_mode: SharedMode = Arc::new(AlternateDummyMode);
        GlobalReadOnlyScope::set_mode_thread_local(local_mode.clone());

        let child = std::thread::spawn(|| {
            let scope = GlobalReadOnlyScope::instance();
            let mode = GlobalReadOnlyScope::get_mode().expect("child should see global mode");
            (scope.sample, mode)
        });

        let (child_sample, child_mode) = child.join().expect("child thread should finish");

        assert_eq!(GlobalReadOnlyScope::instance().sample, "local");
        let parent_mode =
            GlobalReadOnlyScope::get_mode().expect("parent should still see local mode");
        assert!(Arc::ptr_eq(&parent_mode, &local_mode));
        assert_eq!(child_sample, "global");
        assert!(Arc::ptr_eq(&child_mode, &global_mode));
    }
}
