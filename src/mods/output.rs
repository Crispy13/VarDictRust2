use crate::config::Configuration;
use crate::data::{Region, Variant};
use crate::fisher::FisherExact;
use crate::scope::GlobalReadOnlyScope;
use crate::utils::{
    format_half_even, get_rounded_value_to_print, hifreq_fisher_format, nm_fisher_format,
    nm_non_fisher_format, tsv_join, zero_gated_format,
};

fn empty_to_zero(value: &str) -> String {
    if value.is_empty() {
        String::from("0")
    } else {
        value.to_string()
    }
}

#[derive(Clone, Debug, Default)]
pub struct VariantRegion {
    pub variant: Option<Variant>,
    pub region: String,
}

impl VariantRegion {
    pub fn new(variant: Option<Variant>, region: impl Into<String>) -> Self {
        Self {
            variant,
            region: region.into(),
        }
    }
}

/// Ported from: SimpleOutputVariant.java:L14-L198
#[derive(Clone, Debug, Default)]
pub struct SimpleOutputVariant {
    sample: String,
    gene: String,
    chr: String,
    start_position: i32,
    end_position: i32,
    ref_allele: String,
    var_allele: String,
    total_coverage: i32,
    variant_coverage: i32,
    reference_forward_count: i32,
    reference_reverse_count: i32,
    variant_forward_count: i32,
    variant_reverse_count: i32,
    genotype: String,
    frequency: f64,
    bias: String,
    pmean: f64,
    pstd: i32,
    qual: f64,
    qstd: i32,
    mapq: f64,
    qratio: f64,
    hifreq: f64,
    extrafreq: f64,
    shift3: i32,
    msi: f64,
    msint: i32,
    nm: f64,
    hicnt: i32,
    hicov: i32,
    left_sequence: String,
    right_sequence: String,
    region: String,
    var_type: String,
    duprate: f64,
    crispr: i32,
    sv: String,
    pvalue: f64,
    oddratio: String,
    debug: String,
}

impl SimpleOutputVariant {
    /// Ported from: SimpleOutputVariant.SimpleOutputVariant()
    /// Java source: SimpleOutputVariant.java:L42-L103
    pub fn new(variant: Option<&Variant>, region: &Region, sv: &str, position: i32) -> Self {
        let instance = GlobalReadOnlyScope::instance();
        let mut output = Self {
            sample: instance.sample,
            gene: region.gene.clone(),
            chr: region.chr.clone(),
            start_position: position,
            end_position: position,
            bias: String::from("0;0"),
            oddratio: String::from("0"),
            region: format!("{}:{}-{}", region.chr, region.start, region.end),
            sv: empty_to_zero(sv),
            ..Self::default()
        };

        if let Some(variant) = variant {
            output.start_position = variant.start_position;
            output.end_position = variant.end_position;
            output.ref_allele = variant.refallele.clone();
            output.var_allele = variant.varallele.clone();
            output.total_coverage = variant.total_pos_coverage;
            output.variant_coverage = variant.position_coverage;
            output.reference_forward_count = variant.ref_forward_coverage;
            output.reference_reverse_count = variant.ref_reverse_coverage;
            output.variant_forward_count = variant.vars_count_on_forward;
            output.variant_reverse_count = variant.vars_count_on_reverse;
            output.genotype = variant
                .genotype
                .clone()
                .unwrap_or_else(|| String::from("0"));
            output.frequency = variant.frequency;
            output.bias = variant.strand_bias_flag.clone();
            output.pmean = variant.mean_position;
            output.pstd = i32::from(variant.is_at_least_at_2_positions);
            output.qual = variant.mean_quality;
            output.qstd = i32::from(variant.has_at_least_2_diff_qualities);
            output.mapq = variant.mean_mapping_quality;
            output.qratio = variant.high_quality_to_low_quality_ratio;
            output.hifreq = variant.high_quality_reads_frequency;
            output.extrafreq = variant.extra_frequency;
            output.shift3 = variant.shift3;
            output.msi = variant.msi;
            output.msint = variant.msint;
            output.nm = variant.number_of_mismatches;
            output.hicnt = variant.hicnt;
            output.hicov = variant.hicov;
            output.left_sequence = empty_to_zero(&variant.leftseq);
            output.right_sequence = empty_to_zero(&variant.rightseq);
            output.var_type = variant.vartype.clone();
            output.duprate = variant.duprate;
            output.crispr = variant.crispr;
            output.debug = variant.debug.clone();
        }

        if instance.conf.fisher {
            let fisher = if let Some(variant) = variant {
                FisherExact::new(
                    variant.ref_forward_coverage,
                    variant.ref_reverse_coverage,
                    variant.vars_count_on_forward,
                    variant.vars_count_on_reverse,
                )
            } else {
                FisherExact::new(0, 0, 0, 0)
            };
            output.pvalue = fisher.get_p_value();
            output.oddratio = fisher.get_odd_ratio();
        }

        output
    }

    fn simple_variant_38columns(&self) -> String {
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.total_coverage,
            self.variant_coverage,
            self.reference_forward_count,
            self.reference_reverse_count,
            self.variant_forward_count,
            self.variant_reverse_count,
            &self.genotype,
            get_rounded_value_to_print("0.0000", self.frequency),
            &self.bias,
            get_rounded_value_to_print("0.0", self.pmean),
            self.pstd,
            get_rounded_value_to_print("0.0", self.qual),
            self.qstd,
            get_rounded_value_to_print("0.00000", self.pvalue),
            &self.oddratio,
            get_rounded_value_to_print("0.0", self.mapq),
            get_rounded_value_to_print("0.000", self.qratio),
            hifreq_fisher_format(self.hifreq),
            get_rounded_value_to_print("0.0000", self.extrafreq),
            self.shift3,
            get_rounded_value_to_print("0.000", self.msi),
            self.msint,
            nm_fisher_format(self.nm),
            self.hicnt,
            self.hicov,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_type,
            get_rounded_value_to_print("0.00", self.duprate),
            &self.sv,
        )
    }

    fn simple_variant_36columns(&self) -> String {
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.total_coverage,
            self.variant_coverage,
            self.reference_forward_count,
            self.reference_reverse_count,
            self.variant_forward_count,
            self.variant_reverse_count,
            &self.genotype,
            zero_gated_format("0.0000", self.frequency),
            &self.bias,
            zero_gated_format("0.0", self.pmean),
            self.pstd,
            zero_gated_format("0.0", self.qual),
            self.qstd,
            zero_gated_format("0.0", self.mapq),
            zero_gated_format("0.000", self.qratio),
            zero_gated_format("0.0000", self.hifreq),
            zero_gated_format("0.0000", self.extrafreq),
            self.shift3,
            zero_gated_format("0.000", self.msi),
            self.msint,
            nm_non_fisher_format(self.nm),
            self.hicnt,
            self.hicov,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_type,
            zero_gated_format("0.0", self.duprate),
            &self.sv,
        )
    }

    pub fn to_tsv_line(&self, conf: &Configuration) -> String {
        let mut output = if conf.fisher {
            self.simple_variant_38columns()
        } else {
            self.simple_variant_36columns()
        };
        if conf.crispr_cutting_site != 0 {
            output = tsv_join!("\t", output, self.crispr);
        }
        if conf.debug {
            output = tsv_join!("\t", output, &self.debug);
        }
        output
    }
}

/// Ported from: SomaticOutputVariant.java:L14-L319
#[derive(Clone, Debug, Default)]
pub struct SomaticOutputVariant {
    sample: String,
    gene: String,
    chr: String,
    start_position: i32,
    end_position: i32,
    ref_allele: String,
    var_allele: String,
    shift3: i32,
    msi: f64,
    msint: i32,
    left_sequence: String,
    right_sequence: String,
    region: String,
    var_type: String,
    debug: String,
    var1total_coverage: i32,
    var1variant_coverage: i32,
    var1ref_forward_coverage: i32,
    var1ref_reverse_coverage: i32,
    var1variant_forward_count: i32,
    var1variant_reverse_count: i32,
    var1genotype: String,
    var1frequency: f64,
    var1strand_bias_flag: String,
    var1mean_position: f64,
    var1is_at_least_at_2_position: i32,
    pvalue1: f64,
    oddratio1: String,
    var1mean_quality: f64,
    var1has_at_least_2_diff_qualities: i32,
    var1mean_mapping_quality: f64,
    var1high_quality_to_low_quality_ratio: f64,
    var1high_quality_reads_frequency: f64,
    var1extra_frequency: f64,
    var1nm: f64,
    var1duprate: f64,
    var1sv: String,
    var2total_coverage: i32,
    var2variant_coverage: i32,
    var2ref_forward_coverage: i32,
    var2ref_reverse_coverage: i32,
    var2variant_forward_count: i32,
    var2variant_reverse_count: i32,
    var2genotype: String,
    var2frequency: f64,
    var2strand_bias_flag: String,
    var2mean_position: f64,
    var2is_at_least_at_2_position: i32,
    pvalue2: f64,
    oddratio2: String,
    var2mean_quality: f64,
    var2has_at_least_2_diff_qualities: i32,
    var2mean_mapping_quality: f64,
    var2high_quality_to_low_quality_ratio: f64,
    var2high_quality_reads_frequency: f64,
    var2extra_frequency: f64,
    var2nm: f64,
    var2duprate: f64,
    var2sv: String,
    var_label: String,
    pvalue: f64,
    oddratio: String,
}

impl SomaticOutputVariant {
    /// Ported from: SomaticOutputVariant.SomaticOutputVariant()
    /// Java source: SomaticOutputVariant.java:L62-L131
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        begin_variant: Option<&Variant>,
        end_variant: Option<&Variant>,
        tumor_variant: Option<&Variant>,
        normal_variant: Option<&Variant>,
        region: &Region,
        sv1: &str,
        sv2: &str,
        var_label: &str,
    ) -> Self {
        let instance = GlobalReadOnlyScope::instance();
        let mut output = Self {
            sample: instance.sample,
            gene: region.gene.clone(),
            chr: region.chr.clone(),
            region: format!("{}:{}-{}", region.chr, region.start, region.end),
            var_label: var_label.to_string(),
            var1genotype: String::from("0"),
            var1strand_bias_flag: String::from("0"),
            oddratio1: String::from("0"),
            var1sv: empty_to_zero(sv1),
            var2genotype: String::from("0"),
            var2strand_bias_flag: String::from("0"),
            oddratio2: String::from("0"),
            var2sv: empty_to_zero(sv2),
            oddratio: String::from("0"),
            ..Self::default()
        };

        if let Some(begin_variant) = begin_variant {
            output.start_position = begin_variant.start_position;
            output.end_position = begin_variant.end_position;
            output.ref_allele = begin_variant.refallele.clone();
            output.var_allele = begin_variant.varallele.clone();
            output.var_type = begin_variant.vartype.clone();
            output.debug = begin_variant.debug.clone();
        }
        if let Some(end_variant) = end_variant {
            output.shift3 = end_variant.shift3;
            output.msi = end_variant.msi;
            output.msint = end_variant.msint;
            output.left_sequence = empty_to_zero(&end_variant.leftseq);
            output.right_sequence = empty_to_zero(&end_variant.rightseq);
        }
        if let Some(tumor_variant) = tumor_variant {
            output.var1total_coverage = tumor_variant.total_pos_coverage;
            output.var1variant_coverage = tumor_variant.position_coverage;
            output.var1ref_forward_coverage = tumor_variant.ref_forward_coverage;
            output.var1ref_reverse_coverage = tumor_variant.ref_reverse_coverage;
            output.var1variant_forward_count = tumor_variant.vars_count_on_forward;
            output.var1variant_reverse_count = tumor_variant.vars_count_on_reverse;
            output.var1genotype = tumor_variant
                .genotype
                .clone()
                .unwrap_or_else(|| String::from("0"));
            output.var1frequency = tumor_variant.frequency;
            output.var1strand_bias_flag = empty_to_zero(&tumor_variant.strand_bias_flag);
            output.var1mean_position = tumor_variant.mean_position;
            output.var1is_at_least_at_2_position =
                i32::from(tumor_variant.is_at_least_at_2_positions);
            output.var1mean_quality = tumor_variant.mean_quality;
            output.var1has_at_least_2_diff_qualities =
                i32::from(tumor_variant.has_at_least_2_diff_qualities);
            output.var1mean_mapping_quality = tumor_variant.mean_mapping_quality;
            output.var1high_quality_to_low_quality_ratio =
                tumor_variant.high_quality_to_low_quality_ratio;
            output.var1high_quality_reads_frequency = tumor_variant.high_quality_reads_frequency;
            output.var1extra_frequency = tumor_variant.extra_frequency;
            output.var1nm = tumor_variant.number_of_mismatches;
            output.var1duprate = tumor_variant.duprate;
        }
        if let Some(normal_variant) = normal_variant {
            output.var2total_coverage = normal_variant.total_pos_coverage;
            output.var2variant_coverage = normal_variant.position_coverage;
            output.var2ref_forward_coverage = normal_variant.ref_forward_coverage;
            output.var2ref_reverse_coverage = normal_variant.ref_reverse_coverage;
            output.var2variant_forward_count = normal_variant.vars_count_on_forward;
            output.var2variant_reverse_count = normal_variant.vars_count_on_reverse;
            output.var2genotype = normal_variant
                .genotype
                .clone()
                .unwrap_or_else(|| String::from("0"));
            output.var2frequency = normal_variant.frequency;
            output.var2strand_bias_flag = empty_to_zero(&normal_variant.strand_bias_flag);
            output.var2mean_position = normal_variant.mean_position;
            output.var2is_at_least_at_2_position =
                i32::from(normal_variant.is_at_least_at_2_positions);
            output.var2mean_quality = normal_variant.mean_quality;
            output.var2has_at_least_2_diff_qualities =
                i32::from(normal_variant.has_at_least_2_diff_qualities);
            output.var2mean_mapping_quality = normal_variant.mean_mapping_quality;
            output.var2high_quality_to_low_quality_ratio =
                normal_variant.high_quality_to_low_quality_ratio;
            output.var2high_quality_reads_frequency = normal_variant.high_quality_reads_frequency;
            output.var2extra_frequency = normal_variant.extra_frequency;
            output.var2nm = normal_variant.number_of_mismatches;
            output.var2duprate = normal_variant.duprate;
        }

        if instance.conf.fisher {
            output.calculate_fisher_somatic(tumor_variant, normal_variant);
        }

        output
    }

    /// Ported from: SomaticOutputVariant.calculateFisherSomatic()
    /// Java source: SomaticOutputVariant.java:L133-L172
    fn calculate_fisher_somatic(
        &mut self,
        tumor_variant: Option<&Variant>,
        normal_variant: Option<&Variant>,
    ) {
        let fisher1 = if let Some(tumor_variant) = tumor_variant {
            FisherExact::new(
                tumor_variant.ref_forward_coverage,
                tumor_variant.ref_reverse_coverage,
                tumor_variant.vars_count_on_forward,
                tumor_variant.vars_count_on_reverse,
            )
        } else {
            FisherExact::new(0, 0, 0, 0)
        };
        self.pvalue1 = fisher1.get_p_value();
        self.oddratio1 = fisher1.get_odd_ratio();

        let fisher2 = if let Some(normal_variant) = normal_variant {
            FisherExact::new(
                normal_variant.ref_forward_coverage,
                normal_variant.ref_reverse_coverage,
                normal_variant.vars_count_on_forward,
                normal_variant.vars_count_on_reverse,
            )
        } else {
            FisherExact::new(0, 0, 0, 0)
        };
        self.pvalue2 = fisher2.get_p_value();
        self.oddratio2 = fisher2.get_odd_ratio();

        let tref = (self.var1total_coverage - self.var1variant_coverage).max(0);
        let rref = (self.var2total_coverage - self.var2variant_coverage).max(0);
        let fisher = FisherExact::new(
            self.var1variant_coverage,
            tref,
            self.var2variant_coverage,
            rref,
        );
        let pvalue_greater = fisher.get_p_value_greater();
        let pvalue_less = fisher.get_p_value_less();
        self.pvalue = if pvalue_less < pvalue_greater {
            pvalue_less
        } else {
            pvalue_greater
        };
        self.oddratio = fisher.get_odd_ratio();
    }

    fn somatic_variant_61columns(&self) -> String {
        let msi_f = zero_gated_format("0.000", self.msi);
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.var1total_coverage,
            self.var1variant_coverage,
            self.var1ref_forward_coverage,
            self.var1ref_reverse_coverage,
            self.var1variant_forward_count,
            self.var1variant_reverse_count,
            &self.var1genotype,
            get_rounded_value_to_print("0.0000", self.var1frequency),
            &self.var1strand_bias_flag,
            get_rounded_value_to_print("0.0", self.var1mean_position),
            self.var1is_at_least_at_2_position,
            get_rounded_value_to_print("0.0", self.var1mean_quality),
            self.var1has_at_least_2_diff_qualities,
            get_rounded_value_to_print("0.0", self.var1mean_mapping_quality),
            get_rounded_value_to_print("0.000", self.var1high_quality_to_low_quality_ratio),
            get_rounded_value_to_print("0.0000", self.var1high_quality_reads_frequency),
            get_rounded_value_to_print("0.0000", self.var1extra_frequency),
            get_rounded_value_to_print("0.0", self.var1nm.max(0.0)),
            get_rounded_value_to_print("0.00000", self.pvalue1),
            &self.oddratio1,
            self.var2total_coverage,
            self.var2variant_coverage,
            self.var2ref_forward_coverage,
            self.var2ref_reverse_coverage,
            self.var2variant_forward_count,
            self.var2variant_reverse_count,
            &self.var2genotype,
            get_rounded_value_to_print("0.0000", self.var2frequency),
            &self.var2strand_bias_flag,
            get_rounded_value_to_print("0.0", self.var2mean_position),
            self.var2is_at_least_at_2_position,
            get_rounded_value_to_print("0.0", self.var2mean_quality),
            self.var2has_at_least_2_diff_qualities,
            get_rounded_value_to_print("0.0", self.var2mean_mapping_quality),
            get_rounded_value_to_print("0.000", self.var2high_quality_to_low_quality_ratio),
            get_rounded_value_to_print("0.0000", self.var2high_quality_reads_frequency),
            get_rounded_value_to_print("0.0000", self.var2extra_frequency),
            get_rounded_value_to_print("0.0", self.var2nm.max(0.0)),
            get_rounded_value_to_print("0.00000", self.pvalue2),
            &self.oddratio2,
            self.shift3,
            msi_f,
            self.msint,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_label,
            &self.var_type,
            get_rounded_value_to_print("0.0", self.var1duprate),
            &self.var1sv,
            get_rounded_value_to_print("0.0", self.var2duprate),
            &self.var2sv,
            get_rounded_value_to_print("0.00000", self.pvalue),
            &self.oddratio,
        )
    }

    fn somatic_variant_55columns(&self) -> String {
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.var1total_coverage,
            self.var1variant_coverage,
            self.var1ref_forward_coverage,
            self.var1ref_reverse_coverage,
            self.var1variant_forward_count,
            self.var1variant_reverse_count,
            &self.var1genotype,
            zero_gated_format("0.0000", self.var1frequency),
            &self.var1strand_bias_flag,
            zero_gated_format("0.0", self.var1mean_position),
            self.var1is_at_least_at_2_position,
            zero_gated_format("0.0", self.var1mean_quality),
            self.var1has_at_least_2_diff_qualities,
            zero_gated_format("0.0", self.var1mean_mapping_quality),
            zero_gated_format("0.000", self.var1high_quality_to_low_quality_ratio),
            zero_gated_format("0.0000", self.var1high_quality_reads_frequency),
            zero_gated_format("0.0000", self.var1extra_frequency),
            nm_non_fisher_format(self.var1nm),
            self.var2total_coverage,
            self.var2variant_coverage,
            self.var2ref_forward_coverage,
            self.var2ref_reverse_coverage,
            self.var2variant_forward_count,
            self.var2variant_reverse_count,
            &self.var2genotype,
            zero_gated_format("0.0000", self.var2frequency),
            &self.var2strand_bias_flag,
            zero_gated_format("0.0", self.var2mean_position),
            self.var2is_at_least_at_2_position,
            zero_gated_format("0.0", self.var2mean_quality),
            self.var2has_at_least_2_diff_qualities,
            zero_gated_format("0.0", self.var2mean_mapping_quality),
            zero_gated_format("0.000", self.var2high_quality_to_low_quality_ratio),
            zero_gated_format("0.0000", self.var2high_quality_reads_frequency),
            zero_gated_format("0.0000", self.var2extra_frequency),
            nm_non_fisher_format(self.var2nm),
            self.shift3,
            zero_gated_format("0.000", self.msi),
            self.msint,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_label,
            &self.var_type,
            zero_gated_format("0.0", self.var1duprate),
            &self.var1sv,
            zero_gated_format("0.0", self.var2duprate),
            &self.var2sv,
        )
    }

    pub fn to_tsv_line(&self, conf: &Configuration) -> String {
        let mut output = if conf.fisher {
            self.somatic_variant_61columns()
        } else {
            self.somatic_variant_55columns()
        };
        if conf.debug {
            output = tsv_join!("\t", output, &self.debug);
        }
        output
    }
}

/// Ported from: AmpliconOutputVariant.java:L17-L240
#[derive(Clone, Debug, Default)]
pub struct AmpliconOutputVariant {
    sample: String,
    gene: String,
    chr: String,
    start_position: i32,
    end_position: i32,
    ref_allele: String,
    var_allele: String,
    total_coverage: i32,
    variant_coverage: i32,
    reference_forward_count: i32,
    reference_reverse_count: i32,
    variant_forward_count: i32,
    variant_reverse_count: i32,
    genotype: String,
    frequency: f64,
    bias: String,
    pmean: f64,
    pstd: i32,
    qual: f64,
    qstd: i32,
    mapq: f64,
    qratio: f64,
    hifreq: f64,
    extrafreq: f64,
    shift3: i32,
    msi: f64,
    msint: i32,
    nm: f64,
    hicnt: i32,
    hicov: i32,
    left_sequence: String,
    right_sequence: String,
    region: String,
    var_type: String,
    good_variants_count: i32,
    total_variants_count: i32,
    no_coverage: i32,
    amplicon_flag: i32,
    pvalue: f64,
    oddratio: String,
    debug: String,
}

impl AmpliconOutputVariant {
    /// Ported from: AmpliconOutputVariant.AmpliconOutputVariant()
    /// Java source: AmpliconOutputVariant.java:L50-L128
    pub fn new(
        variant: Option<&Variant>,
        region: &Region,
        good_variants: &[VariantRegion],
        bad_variants: &[VariantRegion],
        position: i32,
        gvscnt: i32,
        no_cov: i32,
        flag: bool,
    ) -> Self {
        let instance = GlobalReadOnlyScope::instance();
        let mut output = Self {
            sample: instance.sample,
            gene: region.gene.clone(),
            chr: region.chr.clone(),
            start_position: position,
            end_position: position,
            region: format!("{}:{}-{}", region.chr, position, position),
            genotype: String::new(),
            bias: String::from("0;0"),
            oddratio: String::from("0"),
            ..Self::default()
        };

        if let Some(variant) = variant {
            output.start_position = variant.start_position;
            output.end_position = variant.end_position;
            output.ref_allele = variant.refallele.clone();
            output.var_allele = variant.varallele.clone();
            output.total_coverage = variant.total_pos_coverage;
            output.variant_coverage = variant.position_coverage;
            output.reference_forward_count = variant.ref_forward_coverage;
            output.reference_reverse_count = variant.ref_reverse_coverage;
            output.variant_forward_count = variant.vars_count_on_forward;
            output.variant_reverse_count = variant.vars_count_on_reverse;
            output.genotype = variant
                .genotype
                .clone()
                .unwrap_or_else(|| String::from("0"));
            output.frequency = variant.frequency;
            output.bias = variant.strand_bias_flag.clone();
            output.pmean = variant.mean_position;
            output.pstd = i32::from(variant.is_at_least_at_2_positions);
            output.qual = variant.mean_quality;
            output.qstd = i32::from(variant.has_at_least_2_diff_qualities);
            output.mapq = variant.mean_mapping_quality;
            output.qratio = variant.high_quality_to_low_quality_ratio;
            output.hifreq = variant.high_quality_reads_frequency;
            output.extrafreq = variant.extra_frequency;
            output.shift3 = variant.shift3;
            output.msi = variant.msi;
            output.msint = variant.msint;
            output.nm = variant.number_of_mismatches;
            output.hicnt = variant.hicnt;
            output.hicov = variant.hicov;
            output.left_sequence = empty_to_zero(&variant.leftseq);
            output.right_sequence = empty_to_zero(&variant.rightseq);
            output.var_type = variant.vartype.clone();
            output.good_variants_count = gvscnt;
            output.total_variants_count = gvscnt + i32::try_from(bad_variants.len()).unwrap_or(0);
            output.no_coverage = no_cov;
            output.amplicon_flag = i32::from(flag);
            if let Some(first_good) = good_variants.first() {
                output.region = first_good.region.clone();
            }
            output.debug = variant.debug.clone();
        }

        if instance.conf.fisher {
            let fisher = if let Some(variant) = variant {
                FisherExact::new(
                    variant.ref_forward_coverage,
                    variant.ref_reverse_coverage,
                    variant.vars_count_on_forward,
                    variant.vars_count_on_reverse,
                )
            } else {
                FisherExact::new(0, 0, 0, 0)
            };
            output.pvalue = fisher.get_p_value();
            output.oddratio = fisher.get_odd_ratio();
        }

        if instance.conf.debug && variant.is_some() {
            let mut debug = output.debug.clone();
            for (index, entry) in good_variants.iter().enumerate() {
                let variant = entry
                    .variant
                    .as_ref()
                    .expect("good amplicon variant must exist");
                debug.push_str(&format!(
                    "\tGood{} {}",
                    index,
                    tsv_join!(" ", output.debug_amp_variant(" ", variant), &entry.region)
                ));
            }
            for (index, entry) in bad_variants.iter().enumerate() {
                let variant = entry
                    .variant
                    .as_ref()
                    .expect("bad amplicon debug variant missing in Java path");
                debug.push_str(&format!(
                    "\tBad{} {}",
                    index,
                    tsv_join!(" ", output.debug_amp_variant(" ", variant), &entry.region)
                ));
            }
            output.debug = debug;
        }

        output
    }

    fn amplicon_variant_40columns(&self) -> String {
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.total_coverage,
            self.variant_coverage,
            self.reference_forward_count,
            self.reference_reverse_count,
            self.variant_forward_count,
            self.variant_reverse_count,
            &self.genotype,
            get_rounded_value_to_print("0.0000", self.frequency),
            &self.bias,
            get_rounded_value_to_print("0.0", self.pmean),
            self.pstd,
            get_rounded_value_to_print("0.0", self.qual),
            self.qstd,
            get_rounded_value_to_print("0.00000", self.pvalue),
            &self.oddratio,
            get_rounded_value_to_print("0.0", self.mapq),
            get_rounded_value_to_print("0.000", self.qratio),
            hifreq_fisher_format(self.hifreq),
            get_rounded_value_to_print("0.0000", self.extrafreq),
            self.shift3,
            get_rounded_value_to_print("0.000", self.msi),
            self.msint,
            nm_fisher_format(self.nm),
            self.hicnt,
            self.hicov,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_type,
            self.good_variants_count,
            self.total_variants_count,
            self.no_coverage,
            self.amplicon_flag,
        )
    }

    fn amplicon_variant_38columns(&self) -> String {
        tsv_join!(
            "\t",
            &self.sample,
            &self.gene,
            &self.chr,
            self.start_position,
            self.end_position,
            &self.ref_allele,
            &self.var_allele,
            self.total_coverage,
            self.variant_coverage,
            self.reference_forward_count,
            self.reference_reverse_count,
            self.variant_forward_count,
            self.variant_reverse_count,
            &self.genotype,
            zero_gated_format("0.0000", self.frequency),
            &self.bias,
            zero_gated_format("0.0", self.pmean),
            self.pstd,
            zero_gated_format("0.0", self.qual),
            self.qstd,
            zero_gated_format("0.0", self.mapq),
            zero_gated_format("0.000", self.qratio),
            zero_gated_format("0.0000", self.hifreq),
            zero_gated_format("0.0000", self.extrafreq),
            self.shift3,
            zero_gated_format("0.000", self.msi),
            self.msint,
            nm_non_fisher_format(self.nm),
            self.hicnt,
            self.hicov,
            &self.left_sequence,
            &self.right_sequence,
            &self.region,
            &self.var_type,
            self.good_variants_count,
            self.total_variants_count,
            self.no_coverage,
            self.amplicon_flag,
        )
    }

    pub fn debug_amp_variant(&self, delimiter: &str, variant: &Variant) -> String {
        tsv_join!(
            delimiter,
            variant.total_pos_coverage,
            variant.position_coverage,
            variant.ref_forward_coverage,
            variant.ref_reverse_coverage,
            variant.vars_count_on_forward,
            variant.vars_count_on_reverse,
            variant
                .genotype
                .clone()
                .unwrap_or_else(|| String::from("0")),
            zero_gated_format("0.0000", variant.frequency),
            &variant.strand_bias_flag,
            format_half_even("0.0", variant.mean_position),
            i32::from(variant.is_at_least_at_2_positions),
            format_half_even("0.0", variant.mean_quality),
            i32::from(variant.has_at_least_2_diff_qualities),
            format_half_even("0.0", variant.mean_mapping_quality),
            format_half_even("0.000", variant.high_quality_to_low_quality_ratio),
            zero_gated_format("0.0000", variant.high_quality_reads_frequency),
            zero_gated_format("0.0000", variant.extra_frequency),
        )
    }

    pub fn to_tsv_line(&self, conf: &Configuration) -> String {
        let mut output = if conf.fisher {
            self.amplicon_variant_40columns()
        } else {
            self.amplicon_variant_38columns()
        };
        if conf.debug {
            output = tsv_join!("\t", output, &self.debug);
        }
        output
    }
}