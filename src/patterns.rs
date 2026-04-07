#![allow(non_upper_case_globals)]

use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
// SAMRecord patterns
// Java: Patterns.MC_Z_NUM_S_ANY_NUM_S
pub static ref MC_Z_NUM_S_ANY_NUM_S: Regex = Regex::new(r"\d+S\S*\d+S").unwrap();
}

lazy_static! {
// Variation patterns
// Java: Patterns.BEGIN_DIGITS
pub static ref BEGIN_DIGITS: Regex = Regex::new(r"^(\d+)").unwrap();
// Java: Patterns.UP_NUMBER_END
pub static ref UP_NUMBER_END: Regex = Regex::new(r"\^(\d+)$").unwrap();
// Java: Patterns.BEGIN_MINUS_NUMBER_ANY
pub static ref BEGIN_MINUS_NUMBER_ANY: Regex = Regex::new(r"^-\d+(.*)").unwrap();
// Java: Patterns.BEGIN_MINUS_NUMBER_CARET
pub static ref BEGIN_MINUS_NUMBER_CARET: Regex = Regex::new(r"^-\d+\^").unwrap();
// Java: Patterns.BEGIN_MINUS_NUMBER
pub static ref BEGIN_MINUS_NUMBER: Regex = Regex::new(r"^-(\d+)").unwrap();
// Java: Patterns.MINUS_NUM_NUM
pub static ref MINUS_NUM_NUM: Regex = Regex::new(r"-\d\d").unwrap();
// Java: Patterns.HASH_GROUP_CARET_GROUP
pub static ref HASH_GROUP_CARET_GROUP: Regex = Regex::new(r"#(.+)\^(.+)").unwrap();
}

lazy_static! {
// Sclip patterns
// Java: Patterns.B_A7
pub static ref B_A7: Regex = Regex::new(r"^.AAAAAAA").unwrap();
// Java: Patterns.B_T7
pub static ref B_T7: Regex = Regex::new(r"^.TTTTTTT").unwrap();
}

lazy_static! {
// ATGC patterns
// Java: Patterns.CARET_ATGNC
pub static ref CARET_ATGNC: Regex = Regex::new(r"\^([ATGNC]+)").unwrap();
// Java: Patterns.CARET_ATGC_END
pub static ref CARET_ATGC_END: Regex = Regex::new(r"\^([ATGC]+)$").unwrap();
// Java: Patterns.AMP_ATGC
pub static ref AMP_ATGC: Regex = Regex::new(r"&([ATGC]+)").unwrap();
// Java: Patterns.BEGIN_PLUS_ATGC
pub static ref BEGIN_PLUS_ATGC: Regex = Regex::new(r"^\+([ATGC]+)").unwrap();
// Java: Patterns.HASH_ATGC
pub static ref HASH_ATGC: Regex = Regex::new(r"#([ATGC]+)").unwrap();
// Java: Patterns.ATGSs_AMP_ATGSs_END
pub static ref ATGSs_AMP_ATGSs_END: Regex = Regex::new(r"(\+[ATGC]+)&[ATGC]+$").unwrap();
// Java: Patterns.MINUS_NUMBER_AMP_ATGCs_END
pub static ref MINUS_NUMBER_AMP_ATGCs_END: Regex = Regex::new(r"(-\d+)&[ATGC]+$").unwrap();
// Java: Patterns.MINUS_NUMBER_ATGNC_SV_ATGNC_END
pub static ref MINUS_NUMBER_ATGNC_SV_ATGNC_END: Regex =
    Regex::new(r"^-\d+\^([ATGNC]+)<...\d+>([ATGNC]+)$").unwrap();
// Java: Patterns.BEGIN_ATGC_END
pub static ref BEGIN_ATGC_END: Regex = Regex::new(r"^[ATGC]+$").unwrap();
}

lazy_static! {
// SV patterns
// Java: Patterns.DUP_NUM
pub static ref DUP_NUM: Regex = Regex::new(r"<dup(\d+)").unwrap();
// Java: Patterns.DUP_NUM_ATGC
pub static ref DUP_NUM_ATGC: Regex = Regex::new(r"<dup(\d+)>([ATGC]+)$").unwrap();
// Java: Patterns.INV_NUM
pub static ref INV_NUM: Regex = Regex::new(r"<inv(\d+)").unwrap();
// Java: Patterns.SOME_SV_NUMBERS
pub static ref SOME_SV_NUMBERS: Regex = Regex::new(r"<(...)\d+>").unwrap();
// Java: Patterns.ANY_SV
pub static ref ANY_SV: Regex = Regex::new(r"<(...)>").unwrap();
}

lazy_static! {
// File and columns patterns
// Java: Patterns.SAMPLE_PATTERN
pub static ref SAMPLE_PATTERN: Regex = Regex::new(r"([^\/\._]+).sorted[^\/]*.bam").unwrap();
// Java: Patterns.SAMPLE_PATTERN2
pub static ref SAMPLE_PATTERN2: Regex = Regex::new(r"([^\/]+)[_\.][^\/]*bam").unwrap();
// Java: Patterns.INTEGER_ONLY
pub static ref INTEGER_ONLY: Regex = Regex::new(r"^\d+$").unwrap();
}

lazy_static! {
// CIGAR patterns
// Java: Patterns.BEGIN_NUMBER_S_NUMBER_IorD
pub static ref BEGIN_NUMBER_S_NUMBER_IorD: Regex =
    Regex::new(r"^(\d+)S(\d+)([ID])").unwrap();
// Java: Patterns.NUMBER_IorD_NUMBER_S_END
pub static ref NUMBER_IorD_NUMBER_S_END: Regex =
    Regex::new(r"(\d+)([ID])(\d+)S$").unwrap();
// Java: Patterns.BEGIN_NUMBER_S_NUMBER_M_NUMBER_IorD
pub static ref BEGIN_NUMBER_S_NUMBER_M_NUMBER_IorD: Regex =
    Regex::new(r"^(\d+)S(\d+)M(\d+)([ID])").unwrap();
// Java: Patterns.NUMBER_IorD_NUMBER_M_NUMBER_S_END
pub static ref NUMBER_IorD_NUMBER_M_NUMBER_S_END: Regex =
    Regex::new(r"(\d+)([ID])(\d+)M(\d+)S$").unwrap();
// Java: Patterns.BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M
pub static ref BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M: Regex =
    Regex::new(r"^(\d)M(\d+)([ID])(\d+)M").unwrap();
// Java: Patterns.BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M_
pub static ref BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M_: Regex =
    Regex::new(r"^\dM\d+[ID]\d+M").unwrap();
// Java: Patterns.NUMBER_IorD_DIGIT_M_END
pub static ref NUMBER_IorD_DIGIT_M_END: Regex =
    Regex::new(r"(\d+)([ID])(\d)M$").unwrap();
// Java: Patterns.NUMBER_IorD_NUMBER_M_END
pub static ref NUMBER_IorD_NUMBER_M_END: Regex =
    Regex::new(r"(\d+)([ID])(\d+)M$").unwrap();
// Java: Patterns.D_M_D_DD_M_D_I_D_M_D_DD
pub static ref D_M_D_DD_M_D_I_D_M_D_DD: Regex =
    Regex::new(r"^(.*?)(\d+)M(\d+)D(\d+)M(\d+)I(\d+)M(\d+)D(\d+)M").unwrap();
// Java: Patterns.D_M_D_DD_M_D_I_D_M_D_DD_prim
pub static ref D_M_D_DD_M_D_I_D_M_D_DD_prim: Regex =
    Regex::new(r"(\d+)M(\d+)D(\d+)M(\d+)I(\d+)M(\d+)D(\d+)M").unwrap();
// Java: Patterns.threeDeletionsPattern
pub static ref threeDeletionsPattern: Regex =
    Regex::new(r"^(.*?)(\d+)M(\d+)D(\d+)M(\d+)D(\d+)M(\d+)D(\d+)M").unwrap();
// Java: Patterns.threeIndelsPattern
pub static ref threeIndelsPattern: Regex = Regex::new(
    r"^(.*?)(\d+)M(\d+)([DI])(\d+)M(\d+)([DI])(\d+)M(\d+)([DI])(\d+)M",
)
.unwrap();
// Java: Patterns.DIGM_D_DI_DIGM_D_DI_DIGM_DI_DIGM
pub static ref DIGM_D_DI_DIGM_D_DI_DIGM_DI_DIGM: Regex =
    Regex::new(r"\d+M\d+[DI]\d+M\d+[DI]\d+M\d+[DI]\d+M").unwrap();
// Java: Patterns.DM_DD_DM_DD_DM_DD_DM
pub static ref DM_DD_DM_DD_DM_DD_DM: Regex =
    Regex::new(r"\d+M\d+D\d+M\d+D\d+M\d+D\d+M").unwrap();
// Java: Patterns.DIG_D_DIG_M_DIG_DI_DIGI
pub static ref DIG_D_DIG_M_DIG_DI_DIGI: Regex =
    Regex::new(r"(\d+)D(\d+)M(\d+)([DI])(\d+I)?").unwrap();
// Java: Patterns.DIG_I_DIG_M_DIG_DI_DIGI
pub static ref DIG_I_DIG_M_DIG_DI_DIGI: Regex =
    Regex::new(r"(\d+)I(\d+)M(\d+)([DI])(\d+I)?").unwrap();
// Java: Patterns.NOTDIG_DIG_I_DIG_M_DIG_DI_DIGI
pub static ref NOTDIG_DIG_I_DIG_M_DIG_DI_DIGI: Regex =
    Regex::new(r"(\D)(\d+)I(\d+)M(\d+)([DI])(\d+I)?").unwrap();
// Java: Patterns.DIG_D_DIG_D
pub static ref DIG_D_DIG_D: Regex = Regex::new(r"(\d+)D(\d+)D").unwrap();
// Java: Patterns.DIG_I_DIG_I
pub static ref DIG_I_DIG_I: Regex = Regex::new(r"(\d+)I(\d+)I").unwrap();
// Java: Patterns.BEGIN_ANY_DIG_M_END
pub static ref BEGIN_ANY_DIG_M_END: Regex = Regex::new(r"^(.*?)(\d+)M$").unwrap();
// Java: Patterns.DIG_M_END
pub static ref DIG_M_END: Regex = Regex::new(r"\d+M$").unwrap();
// Java: Patterns.BEGIN_DIG_M
pub static ref BEGIN_DIG_M: Regex = Regex::new(r"^(\d+)M").unwrap();
// Java: Patterns.DIG_S_DIG_M
pub static ref DIG_S_DIG_M: Regex = Regex::new(r"^(\d+)S(\d+)M").unwrap();
// Java: Patterns.DIG_M_DIG_S_END
pub static ref DIG_M_DIG_S_END: Regex = Regex::new(r"\d+M\d+S$").unwrap();
// Java: Patterns.ANY_NUMBER_M_NUMBER_S_END
pub static ref ANY_NUMBER_M_NUMBER_S_END: Regex =
    Regex::new(r"^(.*?)(\d+)M(\d+)S$").unwrap();
// Java: Patterns.BEGIN_NUMBER_D
pub static ref BEGIN_NUMBER_D: Regex = Regex::new(r"^(\d+)D").unwrap();
// Java: Patterns.END_NUMBER_D
pub static ref END_NUMBER_D: Regex = Regex::new(r"(\d+)D$").unwrap();
// Java: Patterns.BEGIN_NUMBER_I
pub static ref BEGIN_NUMBER_I: Regex = Regex::new(r"^(\d+)I").unwrap();
// Java: Patterns.END_NUMBER_I
pub static ref END_NUMBER_I: Regex = Regex::new(r"(\d+)I$").unwrap();
// Java: Patterns.ALIGNED_LENGTH_MND
pub static ref ALIGNED_LENGTH_MND: Regex = Regex::new(r"(\d+)[MND]").unwrap();
// Java: Patterns.ALIGNED_LENGTH_MD
pub static ref ALIGNED_LENGTH_MD: Regex = Regex::new(r"(\d+)[MD=X]").unwrap();
// Java: Patterns.SOFT_CLIPPED
pub static ref SOFT_CLIPPED: Regex = Regex::new(r"(\d+)[MIS]").unwrap();
// Java: Patterns.SA_CIGAR_D_S_5clip
pub static ref SA_CIGAR_D_S_5clip: Regex = Regex::new(r"^\d\d+S").unwrap();
// Java: Patterns.SA_CIGAR_D_S_5clip_GROUP
pub static ref SA_CIGAR_D_S_5clip_GROUP: Regex = Regex::new(r"^(\d\d+)S").unwrap();
// Java: Patterns.SA_CIGAR_D_S_5clip_GROUP_Repl
pub static ref SA_CIGAR_D_S_5clip_GROUP_Repl: Regex = Regex::new(r"^\d+S").unwrap();
// Java: Patterns.SA_CIGAR_D_S_3clip
pub static ref SA_CIGAR_D_S_3clip: Regex = Regex::new(r"\d\dS$").unwrap();
// Java: Patterns.SA_CIGAR_D_S_3clip_GROUP
pub static ref SA_CIGAR_D_S_3clip_GROUP: Regex = Regex::new(r"(\d\d+)S$").unwrap();
// Java: Patterns.SA_CIGAR_D_S_3clip_GROUP_Repl
pub static ref SA_CIGAR_D_S_3clip_GROUP_Repl: Regex = Regex::new(r"\d\d+S$").unwrap();
// Java: Patterns.BEGIN_dig_dig_S_ANY_dig_dig_S_END
pub static ref BEGIN_dig_dig_S_ANY_dig_dig_S_END: Regex =
    Regex::new(r"^\d\dS.*\d\dS$").unwrap();
// Java: Patterns.BEGIN_NUM_S_OR_BEGIN_NUM_H
pub static ref BEGIN_NUM_S_OR_BEGIN_NUM_H: Regex = Regex::new(r"^(\d+)S|^\d+H").unwrap();
// Java: Patterns.END_NUM_S_OR_NUM_H
pub static ref END_NUM_S_OR_NUM_H: Regex = Regex::new(r"(\d+)S$|H$").unwrap();
}

lazy_static! {
    // Exception patterns
    // Java: Patterns.UNABLE_FIND_CONTIG
    pub static ref UNABLE_FIND_CONTIG: Regex =
        Regex::new(r"Unable to find entry for contig").unwrap();
    // Java: Patterns.WRONG_START_OR_END
    pub static ref WRONG_START_OR_END: Regex = Regex::new(r"Malformed query").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patterns_initialize() {
        let patterns: [&Regex; 70] = [
            &MC_Z_NUM_S_ANY_NUM_S,
            &BEGIN_DIGITS,
            &UP_NUMBER_END,
            &BEGIN_MINUS_NUMBER_ANY,
            &BEGIN_MINUS_NUMBER_CARET,
            &BEGIN_MINUS_NUMBER,
            &MINUS_NUM_NUM,
            &HASH_GROUP_CARET_GROUP,
            &B_A7,
            &B_T7,
            &CARET_ATGNC,
            &CARET_ATGC_END,
            &AMP_ATGC,
            &BEGIN_PLUS_ATGC,
            &HASH_ATGC,
            &ATGSs_AMP_ATGSs_END,
            &MINUS_NUMBER_AMP_ATGCs_END,
            &MINUS_NUMBER_ATGNC_SV_ATGNC_END,
            &BEGIN_ATGC_END,
            &DUP_NUM,
            &DUP_NUM_ATGC,
            &INV_NUM,
            &SOME_SV_NUMBERS,
            &ANY_SV,
            &SAMPLE_PATTERN,
            &SAMPLE_PATTERN2,
            &INTEGER_ONLY,
            &BEGIN_NUMBER_S_NUMBER_IorD,
            &NUMBER_IorD_NUMBER_S_END,
            &BEGIN_NUMBER_S_NUMBER_M_NUMBER_IorD,
            &NUMBER_IorD_NUMBER_M_NUMBER_S_END,
            &BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M,
            &BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M_,
            &NUMBER_IorD_DIGIT_M_END,
            &NUMBER_IorD_NUMBER_M_END,
            &D_M_D_DD_M_D_I_D_M_D_DD,
            &D_M_D_DD_M_D_I_D_M_D_DD_prim,
            &threeDeletionsPattern,
            &threeIndelsPattern,
            &DIGM_D_DI_DIGM_D_DI_DIGM_DI_DIGM,
            &DM_DD_DM_DD_DM_DD_DM,
            &DIG_D_DIG_M_DIG_DI_DIGI,
            &DIG_I_DIG_M_DIG_DI_DIGI,
            &NOTDIG_DIG_I_DIG_M_DIG_DI_DIGI,
            &DIG_D_DIG_D,
            &DIG_I_DIG_I,
            &BEGIN_ANY_DIG_M_END,
            &DIG_M_END,
            &BEGIN_DIG_M,
            &DIG_S_DIG_M,
            &DIG_M_DIG_S_END,
            &ANY_NUMBER_M_NUMBER_S_END,
            &BEGIN_NUMBER_D,
            &END_NUMBER_D,
            &BEGIN_NUMBER_I,
            &END_NUMBER_I,
            &ALIGNED_LENGTH_MND,
            &ALIGNED_LENGTH_MD,
            &SOFT_CLIPPED,
            &SA_CIGAR_D_S_5clip,
            &SA_CIGAR_D_S_5clip_GROUP,
            &SA_CIGAR_D_S_5clip_GROUP_Repl,
            &SA_CIGAR_D_S_3clip,
            &SA_CIGAR_D_S_3clip_GROUP,
            &SA_CIGAR_D_S_3clip_GROUP_Repl,
            &BEGIN_dig_dig_S_ANY_dig_dig_S_END,
            &BEGIN_NUM_S_OR_BEGIN_NUM_H,
            &END_NUM_S_OR_NUM_H,
            &UNABLE_FIND_CONTIG,
            &WRONG_START_OR_END,
        ];

        assert_eq!(patterns.len(), 70);
        for pattern in patterns {
            assert!(!pattern.as_str().is_empty());
        }
    }

    #[test]
    fn variation_patterns_match_expected_inputs() {
        let captures = BEGIN_DIGITS.captures("123A").unwrap();
        assert_eq!(&captures[1], "123");

        let captures = HASH_GROUP_CARET_GROUP.captures("#REF^ALT").unwrap();
        assert_eq!(&captures[1], "REF");
        assert_eq!(&captures[2], "ALT");
        assert!(BEGIN_MINUS_NUMBER_CARET.is_match("-12^AT"));
    }

    #[test]
    fn atgc_and_sv_patterns_match_expected_inputs() {
        let captures = MINUS_NUMBER_ATGNC_SV_ATGNC_END
            .captures("-5^ATGC<dup12>GG")
            .unwrap();
        assert_eq!(&captures[1], "ATGC");
        assert_eq!(&captures[2], "GG");

        let captures = DUP_NUM_ATGC.captures("<dup12>ATGC").unwrap();
        assert_eq!(&captures[1], "12");
        assert_eq!(&captures[2], "ATGC");
    }

    #[test]
    fn file_patterns_match_expected_inputs() {
        let captures = SAMPLE_PATTERN.captures("tumor.sorted.markdup.bam").unwrap();
        assert_eq!(&captures[1], "tumor");

        let captures = SAMPLE_PATTERN2.captures("sample_001.bam").unwrap();
        assert_eq!(&captures[1], "sample_001");
        assert!(INTEGER_ONLY.is_match("12345"));
    }

    #[test]
    fn cigar_and_exception_patterns_match_expected_inputs() {
        let captures = BEGIN_NUMBER_S_NUMBER_IorD.captures("12S34I").unwrap();
        assert_eq!(&captures[1], "12");
        assert_eq!(&captures[2], "34");
        assert_eq!(&captures[3], "I");

        assert!(DIG_M_DIG_S_END.is_match("100M5S"));
        assert!(BEGIN_NUM_S_OR_BEGIN_NUM_H.is_match("7H90M"));
        assert!(UNABLE_FIND_CONTIG.is_match("Unable to find entry for contig chr20"));
        assert!(WRONG_START_OR_END.is_match("Malformed query"));
    }
}
