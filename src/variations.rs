use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::hash::Hash;
use std::sync::RwLock;

use once_cell::sync::Lazy;

use crate::config::{Configuration, ADSEED, SEED_2};
use crate::data::{Sclip, SortedStringMap, Variant, Variation, VariationMap, Vars};
use crate::patterns::{B_A7, B_T7};
use crate::scope::GlobalReadOnlyScope;
use crate::utils::{reverse_sequence, substr_with_len};

#[derive(Clone, Debug, Default)]
struct VariationUtilsScope {
    conf: Configuration,
    adaptor_forward: HashMap<String, i32>,
    adaptor_reverse: HashMap<String, i32>,
}

static VARIATION_UTILS_SCOPE: Lazy<RwLock<VariationUtilsScope>> =
    Lazy::new(|| RwLock::new(VariationUtilsScope::default()));

pub trait CountMap<K> {
    fn increment_value(&mut self, key: K, add: i32);
}

impl<K, H> CountMap<K> for HashMap<K, i32, H>
where
    K: Eq + Hash,
    H: BuildHasher,
{
    fn increment_value(&mut self, key: K, add: i32) {
        *self.entry(key).or_insert(0) += add;
    }
}

impl<K> CountMap<K> for BTreeMap<K, i32>
where
    K: Ord,
{
    fn increment_value(&mut self, key: K, add: i32) {
        *self.entry(key).or_insert(0) += add;
    }
}

impl CountMap<String> for crate::data::SortedStringMap<i32> {
    fn increment_value(&mut self, key: String, add: i32) {
        *self.0.entry(key).or_insert(0) += add;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarsType {
    Varn,
    Ref,
    Var,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum VarMaybeArg<'a> {
    #[default]
    None,
    Index(usize),
    Description(&'a str),
}

#[cfg(test)]
fn current_scope() -> VariationUtilsScope {
    with_current_scope(
        |conf, adaptor_forward, adaptor_reverse| VariationUtilsScope {
            conf: conf.clone(),
            adaptor_forward: adaptor_forward.clone(),
            adaptor_reverse: adaptor_reverse.clone(),
        },
    )
}

fn with_current_scope<R>(
    f: impl FnOnce(&Configuration, &HashMap<String, i32>, &HashMap<String, i32>) -> R,
) -> R {
    let mut f = Some(f);
    if let Some(result) = GlobalReadOnlyScope::with_thread_local_instance(|scope| {
        scope.map(|scope| {
            f.take().expect("scope callback should run once")(
                &scope.conf,
                &scope.adaptor_forward,
                &scope.adaptor_reverse,
            )
        })
    }) {
        return result;
    }

    match VARIATION_UTILS_SCOPE.read() {
        Ok(guard) => f.expect("scope callback should be available")(
            &guard.conf,
            &guard.adaptor_forward,
            &guard.adaptor_reverse,
        ),
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            f.expect("scope callback should be available")(
                &guard.conf,
                &guard.adaptor_forward,
                &guard.adaptor_reverse,
            )
        }
    }
}

fn update_scope(scope: VariationUtilsScope) {
    match VARIATION_UTILS_SCOPE.write() {
        Ok(mut guard) => *guard = scope,
        Err(poisoned) => *poisoned.into_inner() = scope,
    }
}

pub fn get_dir(variation: &Variation, dir: bool) -> i32 {
    if dir {
        variation.vars_count_on_reverse
    } else {
        variation.vars_count_on_forward
    }
}

pub fn add_dir(variation: &mut Variation, dir: bool, add: i32) {
    if dir {
        variation.vars_count_on_reverse += add;
    } else {
        variation.vars_count_on_forward += add;
    }
}

pub fn sub_dir(variation: &mut Variation, dir: bool, sub: i32) {
    if dir {
        variation.vars_count_on_reverse -= sub;
    } else {
        variation.vars_count_on_forward -= sub;
    }
}

fn sequence_prefix(sequence: &str, length: i32) -> String {
    String::from_utf8_lossy(&substr_with_len(sequence.as_bytes(), 0, length)).into_owned()
}

fn is_low_complex_seq(sequence: &str) -> bool {
    let length = sequence.len();
    if length == 0 {
        return true;
    }

    let mut nucleotide_count = 0;
    let count_base = |base: u8| {
        sequence
            .as_bytes()
            .iter()
            .filter(|candidate| **candidate == base)
            .count()
    };

    let a = count_base(b'A');
    if a > 0 {
        nucleotide_count += 1;
    }
    if a as f64 / length as f64 > 0.75 {
        return true;
    }

    let t = count_base(b'T');
    if t > 0 {
        nucleotide_count += 1;
    }
    if t as f64 / length as f64 > 0.75 {
        return true;
    }

    let g = count_base(b'G');
    if g > 0 {
        nucleotide_count += 1;
    }
    if g as f64 / length as f64 > 0.75 {
        return true;
    }

    let c = count_base(b'C');
    if c > 0 {
        nucleotide_count += 1;
    }
    if c as f64 / length as f64 > 0.75 {
        return true;
    }

    nucleotide_count < 3
}

/// Ported from: VariationUtils.java:L32-L41
pub fn inc_cnt<K, M>(counts: &mut M, key: K, add: i32)
where
    M: CountMap<K>,
{
    counts.increment_value(key, add);
}

pub fn inc_cnt_sorted_string_map(counts: &mut SortedStringMap<i32>, key: &str, add: i32) {
    if let Some(total) = counts.get_mut(key) {
        *total += add;
    } else {
        counts.insert(key.to_owned(), add);
    }
}

/// Ported from: VariationUtils.java:L49-L63
pub fn strand_bias(forward_count: i32, reverse_count: i32) -> i32 {
    if forward_count + reverse_count <= 12 {
        return if forward_count * reverse_count > 0 {
            2
        } else {
            0
        };
    }

    let (bias, min_bias_reads) = with_current_scope(|conf, _, _| (conf.bias, conf.min_bias_reads));
    let total = (forward_count + reverse_count) as f64;
    let forward_ratio = forward_count as f64 / total;
    let reverse_ratio = reverse_count as f64 / total;

    if forward_ratio >= bias
        && reverse_ratio >= bias
        && forward_count >= min_bias_reads
        && reverse_count >= min_bias_reads
    {
        2
    } else {
        1
    }
}

/// Ported from: VariationUtils.java:L69-L150
pub fn find_conseq(soft_clip: &mut Sclip, dir: i32) -> String {
    if let Some(sequence) = soft_clip.sequence.clone() {
        return sequence;
    }

    let mut total = 0;
    let mut matched = 0;
    let mut candidate_sequence = String::new();
    let mut flag = false;

    if !soft_clip.nt.is_empty() {
        let nt_map = &soft_clip.nt;
        for (position_in_sclip, nucleotide_counts) in nt_map {
            let mut max_count = 0;
            let mut max_quality = 0.0;
            let mut chosen_base: Option<&str> = None;
            let mut total_count = 0;

            for (current_base, current_count) in nucleotide_counts {
                total_count += *current_count;

                let current_quality = soft_clip
                    .seq
                    .get(position_in_sclip)
                    .and_then(|base_map| base_map.get(current_base))
                    .map_or(0.0, |variation| variation.mean_quality);

                let has_higher_quality = soft_clip
                    .seq
                    .get(position_in_sclip)
                    .and_then(|base_map| base_map.get(current_base))
                    .is_some()
                    && current_quality > max_quality;

                if *current_count > max_count || has_higher_quality {
                    max_count = *current_count;
                    chosen_base = Some(current_base.as_str());
                    max_quality = current_quality;
                }
            }

            if *position_in_sclip == 3
                && nt_map.len() >= 6
                && (total_count as f64) / (soft_clip.base.vars_count as f64) < 0.2
                && total_count <= 2
            {
                break;
            }

            if (total_count - max_count > 2 || max_count <= total_count - max_count)
                && (max_count as f64) / (total_count as f64) < 0.8
            {
                if flag {
                    break;
                }
                flag = true;
            }

            total += total_count;
            matched += max_count;
            if let Some(base) = chosen_base {
                candidate_sequence.push_str(base);
            }
        }
    }

    let nt_size: usize = soft_clip.nt.len();
    let mut sequence = if total != 0
        && matched as f64 / total as f64 > 0.9
        && candidate_sequence.len() as f64 / 1.5 > nt_size as f64 - candidate_sequence.len() as f64
        && (candidate_sequence.len() as f64 / nt_size as f64 > 0.8
            || nt_size.saturating_sub(candidate_sequence.len()) < 10
            || candidate_sequence.len() > 25)
    {
        candidate_sequence.clone()
    } else {
        String::new()
    };

    if !sequence.is_empty() && sequence.len() > SEED_2 as usize {
        if B_A7.is_match(&sequence) || B_T7.is_match(&sequence) || is_low_complex_seq(&sequence) {
            soft_clip.used = true;
        }
    }

    if !sequence.is_empty() && sequence.len() >= ADSEED as usize {
        let remove_sequence = with_current_scope(|_, adaptor_forward, adaptor_reverse| {
            if dir == 3 {
                let seed = sequence_prefix(&sequence, ADSEED);
                adaptor_forward.contains_key(&seed)
            } else if dir == 5 {
                let seed = sequence_prefix(&sequence, ADSEED);
                let reversed_seed =
                    String::from_utf8_lossy(&reverse_sequence(seed.as_bytes())).into_owned();
                adaptor_reverse.contains_key(&reversed_seed)
            } else {
                false
            }
        });
        if remove_sequence {
            sequence.clear();
        }
    }

    soft_clip.sequence = Some(sequence.clone());

    if with_current_scope(|conf, _, _| conf.y) {
        eprintln!(
            "  Candidate consensus: {} Reads: {} M: {} T: {} Final: {}",
            candidate_sequence, soft_clip.base.vars_count, matched, total, sequence
        );
    }

    sequence
}

/// Ported from: VariationUtils.java:L159-L170
pub fn join_ref(base_to_position: &HashMap<i32, u8>, from: i32, to: i32) -> String {
    let mut sequence = String::new();
    for position in from..=to {
        if let Some(base) = base_to_position.get(&position) {
            sequence.push(char::from(*base));
        }
    }
    sequence
}

/// Ported from: VariationUtils.java:L178-L191
pub fn join_ref_f64(base_to_position: &HashMap<i32, u8>, from: i32, to: f64) -> String {
    let mut sequence = String::new();
    let mut position = from;
    while (position as f64) < to {
        if let Some(base) = base_to_position.get(&position) {
            sequence.push(char::from(*base));
        }
        position += 1;
    }
    sequence
}

/// Ported from: VariationUtils.java:L201-L217
pub fn join_ref_for_5_lgins(
    base_to_position: &HashMap<i32, u8>,
    from: i32,
    to: i32,
    seq: &str,
    extra: &str,
) -> String {
    let mut sequence = String::new();
    for position in from..=to {
        if to - position < seq.len() as i32 - extra.len() as i32 {
            let index = to - position + extra.len() as i32;
            if let Some(base) = seq.as_bytes().get(index as usize) {
                sequence.push(char::from(*base));
            }
        } else if let Some(base) = base_to_position.get(&position) {
            sequence.push(char::from(*base));
        }
    }
    sequence
}

/// Ported from: VariationUtils.java:L228-L246
pub fn join_ref_for_3_lgins(
    base_to_position: &HashMap<i32, u8>,
    from: i32,
    to: i32,
    shift5: i32,
    seq: &str,
    extra: &str,
) -> String {
    let mut sequence = String::new();
    for position in from..=to {
        let shifted = position - from;
        if shifted >= shift5 && shifted - shift5 < seq.len() as i32 - extra.len() as i32 {
            let index = shifted - shift5 + extra.len() as i32;
            if let Some(base) = seq.as_bytes().get(index as usize) {
                sequence.push(char::from(*base));
            }
        } else if let Some(base) = base_to_position.get(&position) {
            sequence.push(char::from(*base));
        }
    }
    sequence
}

/// Ported from: VariationUtils.java:L253-L263
pub fn adj_cnt(var_to_add: &mut Variation, variant: &Variation) {
    adj_cnt_with_reference(var_to_add, variant, None);
}

/// Ported from: VariationUtils.java:L270-L296
pub fn adj_cnt_with_reference(
    var_to_add: &mut Variation,
    variant: &Variation,
    reference_var: Option<&mut Variation>,
) {
    var_to_add.vars_count += variant.vars_count;
    var_to_add.extracnt += variant.vars_count;
    var_to_add.high_quality_reads_count += variant.high_quality_reads_count;
    var_to_add.low_quality_reads_count += variant.low_quality_reads_count;
    var_to_add.mean_position += variant.mean_position;
    var_to_add.mean_quality += variant.mean_quality;
    var_to_add.mean_mapping_quality += variant.mean_mapping_quality;
    var_to_add.number_of_mismatches += variant.number_of_mismatches;
    var_to_add.pstd = true;
    var_to_add.qstd = true;
    add_dir(var_to_add, true, get_dir(variant, true));
    add_dir(var_to_add, false, get_dir(variant, false));

    if with_current_scope(|conf, _, _| conf.y) {
        let ref_count = reference_var
            .as_ref()
            .filter(|reference| reference.vars_count != 0)
            .map_or_else(
                || String::from("NA"),
                |reference| reference.vars_count.to_string(),
            );
        eprintln!(
            "    AdjCnt: '+' {} {} {} {} Ref: {}",
            var_to_add.vars_count,
            variant.vars_count,
            get_dir(var_to_add, false),
            get_dir(variant, false),
            ref_count
        );
        eprintln!(
            "    AdjCnt: '-' {} {} {} {} Ref: {}",
            var_to_add.vars_count,
            variant.vars_count,
            get_dir(var_to_add, true),
            get_dir(variant, true),
            ref_count
        );
    }

    if let Some(reference_var) = reference_var {
        reference_var.vars_count -= variant.vars_count;
        reference_var.high_quality_reads_count -= variant.high_quality_reads_count;
        reference_var.low_quality_reads_count -= variant.low_quality_reads_count;
        reference_var.mean_position -= variant.mean_position;
        reference_var.mean_quality -= variant.mean_quality;
        reference_var.mean_mapping_quality -= variant.mean_mapping_quality;
        reference_var.number_of_mismatches -= variant.number_of_mismatches;
        sub_dir(reference_var, true, get_dir(variant, true));
        sub_dir(reference_var, false, get_dir(variant, false));
        correct_cnt(reference_var);
    }
}

/// Ported from: VariationUtils.java:L302-L315
pub fn correct_cnt(var_to_correct: &mut Variation) {
    if var_to_correct.vars_count < 0 {
        var_to_correct.vars_count = 0;
    }
    if var_to_correct.high_quality_reads_count < 0 {
        var_to_correct.high_quality_reads_count = 0;
    }
    if var_to_correct.low_quality_reads_count < 0 {
        var_to_correct.low_quality_reads_count = 0;
    }
    if var_to_correct.mean_position < 0.0 {
        var_to_correct.mean_position = 0.0;
    }
    if var_to_correct.mean_quality < 0.0 {
        var_to_correct.mean_quality = 0.0;
    }
    if var_to_correct.mean_mapping_quality < 0.0 {
        var_to_correct.mean_mapping_quality = 0.0;
    }
    if get_dir(var_to_correct, true) < 0 {
        add_dir(var_to_correct, true, -get_dir(var_to_correct, true));
    }
    if get_dir(var_to_correct, false) < 0 {
        add_dir(var_to_correct, false, -get_dir(var_to_correct, false));
    }
}

/// Ported from: VariationUtils.java:L324-L355
pub fn get_var_maybe<'a>(
    aligned_variants: &'a HashMap<i32, Vars>,
    key: i32,
    vars_type: VarsType,
    arg: VarMaybeArg<'_>,
) -> Option<&'a Variant> {
    aligned_variants
        .get(&key)
        .and_then(|vars| get_var_maybe_from_vars(vars, vars_type, arg))
}

/// Ported from: VariationUtils.java:L362-L378
pub fn get_var_maybe_from_vars<'a>(
    vars: &'a Vars,
    vars_type: VarsType,
    arg: VarMaybeArg<'_>,
) -> Option<&'a Variant> {
    match vars_type {
        VarsType::Var => match arg {
            VarMaybeArg::Index(index) => vars.variants.get(index),
            VarMaybeArg::Description(_) | VarMaybeArg::None => None,
        },
        VarsType::Varn => match arg {
            VarMaybeArg::Description(description) => {
                vars.var_description_string_to_variants.get(description)
            }
            VarMaybeArg::Index(_) | VarMaybeArg::None => None,
        },
        VarsType::Ref => vars.reference_variant.as_ref(),
    }
}

/// Ported from: VariationUtils.java:L385-L392
pub fn get_or_put_vars(map: &mut HashMap<i32, Vars>, position: i32) -> &mut Vars {
    map.entry(position).or_default()
}

/// Ported from: VariationUtils.java:L400-L414
pub fn get_variation_from_seq<S>(soft_clip: &mut Sclip, idx: i32, base: S) -> &mut Variation
where
    S: AsRef<str> + Into<String>,
{
    let sequence_map = &mut soft_clip.seq;
    let base_map = sequence_map
        .entry(idx)
        .or_insert_with(crate::data::SortedStringMap::new);

    let base_ref = base.as_ref();
    if base_map.contains_key(base_ref) {
        return base_map
            .get_mut(base_ref)
            .expect("variation entry should exist after contains_key");
    }

    let base = base.into();
    base_map.entry(base).or_default()
}

/// Ported from: VariationUtils.java:L424-L438
pub fn get_variation<S, H>(
    hash: &mut HashMap<i32, VariationMap, H>,
    start: i32,
    description_string: S,
) -> &mut Variation
where
    S: AsRef<str> + Into<String>,
    H: BuildHasher,
{
    let map = hash.entry(start).or_default();

    let description_ref = description_string.as_ref();
    if map.entries.contains_key(description_ref) {
        return map
            .entries
            .get_mut(description_ref)
            .expect("variation entry should exist after contains_key");
    }

    let description_string = description_string.into();
    map.entries.entry(description_string).or_default()
}

/// Ported from: VariationUtils.java:L446-L458
pub fn get_variation_maybe<H>(
    hash: &HashMap<i32, VariationMap, H>,
    start: i32,
    ref_base: Option<u8>,
) -> Option<&Variation>
where
    H: BuildHasher,
{
    let ref_base = ref_base?;
    let description_string = [ref_base];
    let description_string = std::str::from_utf8(&description_string)
        .expect("single-base variation keys should be ASCII");
    hash.get(&start)
        .and_then(|map| map.entries.get(description_string))
}

/// Mutable version of get_variation_maybe — used by processInsertion's subCnt path.
pub fn get_variation_maybe_mut<H>(
    hash: &mut HashMap<i32, VariationMap, H>,
    start: i32,
    ref_base: Option<u8>,
) -> Option<&mut Variation>
where
    H: BuildHasher,
{
    let ref_base = ref_base?;
    let description_string = [ref_base];
    let description_string = std::str::from_utf8(&description_string)
        .expect("single-base variation keys should be ASCII");
    hash.get_mut(&start)
        .and_then(|map| map.entries.get_mut(description_string))
}

/// Ported from: VariationUtils.java:L460-L464
pub fn is_has_and_equals_base(ch1: u8, reference: &HashMap<i32, u8>, index: i32) -> bool {
    reference.get(&index).is_some_and(|refc| *refc == ch1)
}

/// Ported from: VariationUtils.java:L466-L473
pub fn is_has_and_equals_index(index: i32, reference: &HashMap<i32, u8>, index2: i32) -> bool {
    match (reference.get(&index), reference.get(&index2)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// Ported from: VariationUtils.java:L475-L480
pub fn is_has_and_equals_str(
    reference: &HashMap<i32, u8>,
    index1: i32,
    string: &str,
    index2: i32,
) -> bool {
    let Some(reference_base) = reference.get(&index1) else {
        return false;
    };
    let Some(compare_index) = usize::try_from(index2).ok() else {
        return false;
    };
    string
        .as_bytes()
        .get(compare_index)
        .is_some_and(|candidate| reference_base == candidate)
}

/// Ported from: VariationUtils.java:L482-L487
pub fn is_has_and_not_equals_base(ch1: u8, reference: &HashMap<i32, u8>, index: i32) -> bool {
    reference.get(&index).is_some_and(|refc| *refc != ch1)
}

/// Ported from: VariationUtils.java:L489-L497
pub fn is_has_and_not_equals_str(
    reference: &HashMap<i32, u8>,
    index1: i32,
    string: &str,
    index2: i32,
) -> bool {
    let Some(reference_base) = reference.get(&index1) else {
        return false;
    };
    let Some(compare_index) = usize::try_from(index2).ok() else {
        return false;
    };
    string
        .as_bytes()
        .get(compare_index)
        .is_some_and(|candidate| reference_base != candidate)
}

pub fn is_reference_mismatch_and_not_n(
    reference: &HashMap<i32, u8>,
    index1: i32,
    string: &str,
    index2: i32,
) -> bool {
    let Some(reference_base) = reference.get(&index1) else {
        return false;
    };
    if *reference_base == b'N' {
        return false;
    }
    let Some(compare_index) = usize::try_from(index2).ok() else {
        return false;
    };
    string
        .as_bytes()
        .get(compare_index)
        .is_some_and(|candidate| reference_base != candidate)
}

/// Ported from: VariationUtils.java:L499-L505
pub fn is_equals(ch1: Option<u8>, ch2: Option<u8>) -> bool {
    ch1 == ch2
}

/// Ported from: VariationUtils.java:L507-L510
pub fn is_not_equals(ch1: Option<u8>, ch2: Option<u8>) -> bool {
    !is_equals(ch1, ch2)
}

pub fn configure_variation_utils_scope(
    conf: Configuration,
    adaptor_forward: HashMap<String, i32>,
    adaptor_reverse: HashMap<String, i32>,
) {
    update_scope(VariationUtilsScope {
        conf,
        adaptor_forward,
        adaptor_reverse,
    });
}

pub fn clear_variation_utils_scope() {
    update_scope(VariationUtilsScope::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{SortedIntMap, SortedStringMap, VariationEntries};
    use std::sync::Mutex;

    static TEST_SCOPE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn init_sclip() -> Sclip {
        let mut sclip = Sclip::default();
        sclip.base.vars_count = 2;
        sclip.nt = SortedIntMap::from([
            (0, SortedStringMap::from([(String::from("C"), 2)])),
            (1, SortedStringMap::from([(String::from("T"), 2)])),
            (2, SortedStringMap::from([(String::from("A"), 2)])),
            (3, SortedStringMap::from([(String::from("A"), 2)])),
            (4, SortedStringMap::from([(String::from("A"), 2)])),
            (5, SortedStringMap::from([(String::from("T"), 2)])),
            (6, SortedStringMap::from([(String::from("C"), 2)])),
        ]);
        sclip.seq = SortedIntMap::from([
            (
                0,
                SortedStringMap::from([(
                    String::from("C"),
                    Variation {
                        mean_quality: 82.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                1,
                SortedStringMap::from([(
                    String::from("T"),
                    Variation {
                        mean_quality: 82.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                2,
                SortedStringMap::from([(
                    String::from("A"),
                    Variation {
                        mean_quality: 78.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                3,
                SortedStringMap::from([(
                    String::from("A"),
                    Variation {
                        mean_quality: 53.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                4,
                SortedStringMap::from([(
                    String::from("A"),
                    Variation {
                        mean_quality: 78.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                5,
                SortedStringMap::from([(
                    String::from("T"),
                    Variation {
                        mean_quality: 73.0,
                        ..Variation::default()
                    },
                )]),
            ),
            (
                6,
                SortedStringMap::from([(
                    String::from("C"),
                    Variation {
                        mean_quality: 78.0,
                        ..Variation::default()
                    },
                )]),
            ),
        ]);
        sclip
    }

    #[test]
    fn strand_bias_matches_java_thresholds() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        clear_variation_utils_scope();

        assert_eq!(strand_bias(1, 0), 0);
        assert_eq!(strand_bias(2, 2), 2);
        assert_eq!(strand_bias(20, 1), 1);
    }

    #[test]
    fn correct_cnt_clamps_negative_fields_to_zero() {
        let mut variation = Variation {
            vars_count: -3,
            vars_count_on_forward: -2,
            vars_count_on_reverse: -4,
            mean_position: -1.0,
            mean_quality: -2.5,
            mean_mapping_quality: -3.5,
            low_quality_reads_count: -7,
            high_quality_reads_count: -8,
            ..Variation::default()
        };

        correct_cnt(&mut variation);

        assert_eq!(variation.vars_count, 0);
        assert_eq!(variation.vars_count_on_forward, 0);
        assert_eq!(variation.vars_count_on_reverse, 0);
        assert_eq!(variation.mean_position, 0.0);
        assert_eq!(variation.mean_quality, 0.0);
        assert_eq!(variation.mean_mapping_quality, 0.0);
        assert_eq!(variation.low_quality_reads_count, 0);
        assert_eq!(variation.high_quality_reads_count, 0);
    }

    #[test]
    fn join_ref_skips_missing_positions_like_java() {
        let reference = HashMap::from([(1, b'A'), (3, b'C'), (4, b'G')]);

        assert_eq!(join_ref(&reference, 1, 4), "ACG");
        assert_eq!(join_ref_f64(&reference, 1, 3.5), "AC");
    }

    #[test]
    fn find_conseq_without_adapter_matches_java_fixture() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = init_sclip();
        assert_eq!(find_conseq(&mut sclip, 0), "CTAAATC");
        assert_eq!(sclip.sequence.as_deref(), Some("CTAAATC"));

        clear_variation_utils_scope();
    }

    #[test]
    fn find_conseq_rejects_forward_adapter_seed() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(
            Configuration::default(),
            HashMap::from([(String::from("CTAAAT"), 1)]),
            HashMap::new(),
        );

        let mut sclip = init_sclip();
        assert_eq!(find_conseq(&mut sclip, 3), "");
        assert_eq!(sclip.sequence.as_deref(), Some(""));

        clear_variation_utils_scope();
    }

    #[test]
    fn find_conseq_rejects_reverse_adapter_seed() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(
            Configuration::default(),
            HashMap::new(),
            HashMap::from([(String::from("TAAATC"), 1)]),
        );

        let mut sclip = init_sclip();
        assert_eq!(find_conseq(&mut sclip, 5), "");
        assert_eq!(sclip.sequence.as_deref(), Some(""));

        clear_variation_utils_scope();
    }

    #[test]
    fn get_var_maybe_reads_all_supported_sources() {
        let variant = Variant {
            description_string: String::from("A"),
            ..Variant::default()
        };
        let vars = Vars {
            reference_variant: Some(variant.clone()),
            variants: vec![variant.clone()],
            var_description_string_to_variants: BTreeMap::from([(String::from("A"), variant)]),
            sv: String::new(),
        };
        let aligned = HashMap::from([(7, vars)]);

        assert!(get_var_maybe(&aligned, 7, VarsType::Ref, VarMaybeArg::None).is_some());
        assert!(get_var_maybe(&aligned, 7, VarsType::Var, VarMaybeArg::Index(0)).is_some());
        assert!(
            get_var_maybe(&aligned, 7, VarsType::Varn, VarMaybeArg::Description("A")).is_some()
        );
    }

    #[test]
    fn inc_cnt_supports_hash_and_btree_maps() {
        let mut hash_counts = HashMap::new();
        inc_cnt(&mut hash_counts, 4, 2);
        inc_cnt(&mut hash_counts, 4, 3);
        assert_eq!(hash_counts.get(&4), Some(&5));

        let mut tree_counts = BTreeMap::new();
        inc_cnt(&mut tree_counts, String::from("A"), 1);
        inc_cnt(&mut tree_counts, String::from("A"), 2);
        assert_eq!(tree_counts.get("A"), Some(&3));
    }

    #[test]
    fn helper_comparisons_are_null_safe() {
        let reference = HashMap::from([(10, b'A'), (11, b'C'), (12, b'N')]);

        assert!(is_has_and_equals_base(b'A', &reference, 10));
        assert!(is_has_and_equals_index(10, &reference, 10));
        assert!(is_has_and_equals_str(&reference, 11, "CC", 0));
        assert!(is_has_and_not_equals_base(b'T', &reference, 10));
        assert!(is_has_and_not_equals_str(&reference, 11, "AA", 0));
        assert!(is_reference_mismatch_and_not_n(&reference, 11, "AA", 0));
        assert!(!is_reference_mismatch_and_not_n(&reference, 12, "AA", 0));
        assert!(!is_reference_mismatch_and_not_n(&reference, 99, "AA", 0));
        assert!(is_equals(Some(b'A'), Some(b'A')));
        assert!(is_equals(None, None));
        assert!(is_not_equals(Some(b'A'), None));
    }

    #[test]
    fn get_variation_helpers_lazy_initialize_storage() {
        let mut variation_hash = HashMap::new();
        let variation = get_variation(&mut variation_hash, 42, "+INS");
        variation.vars_count = 5;
        assert_eq!(variation_hash[&42].entries["+INS"].vars_count, 5);

        let mut sclip = Sclip::default();
        let seq_variation = get_variation_from_seq(&mut sclip, 1, "A");
        seq_variation.mean_quality = 7.0;
        assert_eq!(
            sclip
                .seq
                .get(&1)
                .and_then(|bases| bases.get("A"))
                .map(|variation| variation.mean_quality),
            Some(7.0)
        );
    }

    #[test]
    fn join_ref_large_insertion_helpers_match_java_indexing() {
        let reference = HashMap::from([(1, b'A'), (2, b'C'), (3, b'G'), (4, b'T')]);

        assert_eq!(join_ref_for_5_lgins(&reference, 1, 4, "TTAA", "T"), "AAAT");
        assert_eq!(
            join_ref_for_3_lgins(&reference, 1, 4, 1, "TTAA", "T"),
            "ATAA"
        );
    }

    #[test]
    fn adj_cnt_moves_counts_and_corrects_reference() {
        let mut var_to_add = Variation::default();
        let variant = Variation {
            vars_count: 3,
            vars_count_on_forward: 1,
            vars_count_on_reverse: 2,
            mean_position: 4.0,
            mean_quality: 5.0,
            mean_mapping_quality: 6.0,
            number_of_mismatches: 7.0,
            low_quality_reads_count: 8,
            high_quality_reads_count: 9,
            ..Variation::default()
        };
        let mut reference = Variation {
            vars_count: 1,
            vars_count_on_forward: 0,
            vars_count_on_reverse: 1,
            mean_position: 1.0,
            mean_quality: 1.0,
            mean_mapping_quality: 1.0,
            number_of_mismatches: 1.0,
            low_quality_reads_count: 1,
            high_quality_reads_count: 1,
            ..Variation::default()
        };

        adj_cnt_with_reference(&mut var_to_add, &variant, Some(&mut reference));

        assert_eq!(var_to_add.vars_count, 3);
        assert_eq!(var_to_add.extracnt, 3);
        assert_eq!(var_to_add.vars_count_on_forward, 1);
        assert_eq!(var_to_add.vars_count_on_reverse, 2);
        assert_eq!(reference.vars_count, 0);
        assert_eq!(reference.vars_count_on_forward, 0);
        assert_eq!(reference.vars_count_on_reverse, 0);
    }

    #[test]
    fn get_variation_maybe_looks_up_reference_base_strings() {
        let mut entries = VariationEntries::default();
        entries.insert(String::from("A"), Variation::default());

        let map = VariationMap { entries, sv: None };
        let hash = HashMap::from([(12, map)]);

        assert!(get_variation_maybe(&hash, 12, Some(b'A')).is_some());
        assert!(get_variation_maybe(&hash, 12, Some(b'T')).is_none());
        assert!(get_variation_maybe(&hash, 12, None).is_none());
    }

    #[test]
    fn get_or_put_vars_creates_default_container_once() {
        let mut vars_map = HashMap::new();
        get_or_put_vars(&mut vars_map, 5).sv = String::from("DEL");
        let existing = get_or_put_vars(&mut vars_map, 5);
        assert_eq!(existing.sv, "DEL");
    }

    #[test]
    fn low_complex_sequence_matches_java_examples() {
        assert!(is_low_complex_seq("AAAAAAAAA"));
        assert!(is_low_complex_seq("ATATATATATAT"));
        assert!(is_low_complex_seq("CCCCCCCCGA"));
        assert!(!is_low_complex_seq("ACGTACGTACGT"));
        assert!(!is_low_complex_seq("CCGTAACGGGGT"));
    }

    #[test]
    fn configure_and_clear_variation_utils_scope_updates_runtime_state() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");

        let configuration = Configuration {
            bias: 0.2,
            min_bias_reads: 4,
            ..Configuration::default()
        };
        configure_variation_utils_scope(
            configuration,
            HashMap::from([(String::from("AAAAAA"), 1)]),
            HashMap::from([(String::from("TTTTTT"), 1)]),
        );

        let scope = current_scope();
        assert_eq!(scope.conf.bias, 0.2);
        assert_eq!(scope.conf.min_bias_reads, 4);
        assert!(scope.adaptor_forward.contains_key("AAAAAA"));
        assert!(scope.adaptor_reverse.contains_key("TTTTTT"));

        clear_variation_utils_scope();
        let cleared_scope = current_scope();
        assert!(cleared_scope.adaptor_forward.is_empty());
        assert!(cleared_scope.adaptor_reverse.is_empty());
        assert_eq!(cleared_scope.conf.bias, Configuration::default().bias);
    }

    #[test]
    fn current_scope_prefers_thread_local_global_read_only_scope_configuration() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");

        clear_variation_utils_scope();
        GlobalReadOnlyScope::clear_thread_local();

        let configuration = Configuration {
            bias: 0.25,
            min_bias_reads: 4,
            ..Configuration::default()
        };

        GlobalReadOnlyScope::init_thread_local(
            configuration,
            HashMap::new(),
            "test_sample",
            None,
            None,
            HashMap::from([(String::from("AAAAAA"), 1)]),
            HashMap::from([(String::from("TTTTTT"), 1)]),
        );

        let scope = current_scope();
        assert_eq!(scope.conf.bias, 0.25);
        assert_eq!(scope.conf.min_bias_reads, 4);
        assert!(scope.adaptor_forward.contains_key("AAAAAA"));
        assert!(scope.adaptor_reverse.contains_key("TTTTTT"));

        GlobalReadOnlyScope::clear_thread_local();
        clear_variation_utils_scope();
    }

    #[test]
    fn find_conseq_marks_low_complex_seed_as_used() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = Sclip::default();
        sclip.base.vars_count = 2;
        sclip.nt = SortedIntMap::from([
            (0, SortedStringMap::from([(String::from("A"), 2)])),
            (1, SortedStringMap::from([(String::from("A"), 2)])),
            (2, SortedStringMap::from([(String::from("A"), 2)])),
            (3, SortedStringMap::from([(String::from("A"), 2)])),
            (4, SortedStringMap::from([(String::from("A"), 2)])),
            (5, SortedStringMap::from([(String::from("A"), 2)])),
            (6, SortedStringMap::from([(String::from("A"), 2)])),
            (7, SortedStringMap::from([(String::from("A"), 2)])),
            (8, SortedStringMap::from([(String::from("A"), 2)])),
            (9, SortedStringMap::from([(String::from("A"), 2)])),
            (10, SortedStringMap::from([(String::from("A"), 2)])),
            (11, SortedStringMap::from([(String::from("A"), 2)])),
            (12, SortedStringMap::from([(String::from("A"), 2)])),
        ]);
        sclip.seq = (0..=12)
            .map(|index| {
                (
                    index,
                    SortedStringMap::from([(
                        String::from("A"),
                        Variation {
                            mean_quality: 50.0,
                            ..Variation::default()
                        },
                    )]),
                )
            })
            .collect::<SortedIntMap<_>>();

        assert_eq!(find_conseq(&mut sclip, 0), "AAAAAAAAAAAAA");
        assert!(sclip.used);

        clear_variation_utils_scope();
    }

    #[test]
    fn join_ref_f64_excludes_upper_bound_like_java() {
        let reference = HashMap::from([(4, b'T'), (5, b'G'), (6, b'C')]);
        assert_eq!(join_ref_f64(&reference, 4, 6.0), "TG");
    }

    #[test]
    fn get_var_maybe_returns_none_for_missing_entries() {
        let vars = Vars::default();
        assert!(get_var_maybe_from_vars(&vars, VarsType::Var, VarMaybeArg::Index(0)).is_none());
        assert!(
            get_var_maybe_from_vars(&vars, VarsType::Varn, VarMaybeArg::Description("A")).is_none()
        );
    }

    #[test]
    fn comparison_helpers_return_false_for_missing_indexes() {
        let reference = HashMap::from([(1, b'A')]);

        assert!(!is_has_and_equals_base(b'A', &reference, 2));
        assert!(!is_has_and_equals_index(1, &reference, 2));
        assert!(!is_has_and_equals_str(&reference, 2, "AA", 0));
        assert!(!is_has_and_not_equals_base(b'T', &reference, 2));
        assert!(!is_has_and_not_equals_str(&reference, 2, "AA", 0));
    }

    #[test]
    fn adj_cnt_without_reference_only_updates_target() {
        let mut var_to_add = Variation::default();
        let variant = Variation {
            vars_count: 2,
            vars_count_on_forward: 2,
            mean_position: 3.0,
            ..Variation::default()
        };

        adj_cnt(&mut var_to_add, &variant);

        assert_eq!(var_to_add.vars_count, 2);
        assert_eq!(var_to_add.extracnt, 2);
        assert_eq!(var_to_add.vars_count_on_forward, 2);
    }

    #[test]
    fn get_variation_from_seq_reuses_existing_entry() {
        let mut sclip = Sclip::default();
        get_variation_from_seq(&mut sclip, 2, "G").vars_count = 1;
        get_variation_from_seq(&mut sclip, 2, "G").vars_count += 1;

        assert_eq!(
            sclip
                .seq
                .get(&2)
                .and_then(|bases| bases.get("G"))
                .map(|variation| variation.vars_count),
            Some(2)
        );
    }

    #[test]
    fn get_variation_reuses_existing_entry() {
        let mut hash = HashMap::new();
        get_variation(&mut hash, 8, "DEL").vars_count = 4;
        get_variation(&mut hash, 8, "DEL").vars_count += 1;

        assert_eq!(hash[&8].entries["DEL"].vars_count, 5);
    }

    #[test]
    fn find_conseq_returns_cached_sequence_when_available() {
        let mut sclip = Sclip {
            sequence: Some(String::from("CACHED")),
            ..Sclip::default()
        };

        assert_eq!(find_conseq(&mut sclip, 0), "CACHED");
    }

    #[test]
    fn inc_cnt_overwrites_existing_total_with_addition() {
        let mut counts = HashMap::from([(String::from("A"), 4)]);
        inc_cnt(&mut counts, String::from("A"), -1);
        assert_eq!(counts.get("A"), Some(&3));
    }

    #[test]
    fn find_conseq_returns_empty_when_consensus_is_weak() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = Sclip::default();
        sclip.base.vars_count = 10;
        sclip.nt = SortedIntMap::from([
            (
                0,
                SortedStringMap::from([(String::from("A"), 4), (String::from("C"), 4)]),
            ),
            (
                1,
                SortedStringMap::from([(String::from("A"), 4), (String::from("C"), 4)]),
            ),
        ]);
        sclip.seq = SortedIntMap::from([
            (
                0,
                SortedStringMap::from([
                    (
                        String::from("A"),
                        Variation {
                            mean_quality: 10.0,
                            ..Variation::default()
                        },
                    ),
                    (
                        String::from("C"),
                        Variation {
                            mean_quality: 12.0,
                            ..Variation::default()
                        },
                    ),
                ]),
            ),
            (
                1,
                SortedStringMap::from([
                    (
                        String::from("A"),
                        Variation {
                            mean_quality: 10.0,
                            ..Variation::default()
                        },
                    ),
                    (
                        String::from("C"),
                        Variation {
                            mean_quality: 12.0,
                            ..Variation::default()
                        },
                    ),
                ]),
            ),
        ]);

        assert_eq!(find_conseq(&mut sclip, 0), "");
        clear_variation_utils_scope();
    }

    #[test]
    fn comparison_helpers_match_option_equality() {
        assert!(is_equals(Some(b'G'), Some(b'G')));
        assert!(!is_equals(Some(b'G'), Some(b'T')));
        assert!(!is_equals(Some(b'G'), None));
        assert!(is_not_equals(Some(b'G'), Some(b'T')));
    }

    #[test]
    fn join_ref_for_insertion_helpers_fall_back_to_reference_when_extra_consumes_seq() {
        let reference = HashMap::from([(1, b'A'), (2, b'C'), (3, b'G')]);
        assert_eq!(join_ref_for_5_lgins(&reference, 1, 3, "AG", "AG"), "ACG");
        assert_eq!(join_ref_for_3_lgins(&reference, 1, 3, 0, "AG", "AG"), "ACG");
    }

    #[test]
    fn get_variation_maybe_returns_none_for_missing_position() {
        let hash: HashMap<i32, VariationMap> = HashMap::new();
        assert!(get_variation_maybe(&hash, 99, Some(b'A')).is_none());
    }

    #[test]
    fn get_or_put_vars_preserves_existing_variants() {
        let mut vars_map = HashMap::from([(
            3,
            Vars {
                reference_variant: None,
                variants: vec![Variant::default()],
                var_description_string_to_variants: BTreeMap::new(),
                sv: String::new(),
            },
        )]);
        assert_eq!(get_or_put_vars(&mut vars_map, 3).variants.len(), 1);
    }

    #[test]
    fn configure_variation_utils_scope_affects_strand_bias_thresholds() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        let configuration = Configuration {
            bias: 0.25,
            min_bias_reads: 3,
            ..Configuration::default()
        };
        configure_variation_utils_scope(configuration, HashMap::new(), HashMap::new());

        assert_eq!(strand_bias(3, 9), 2);
        assert_eq!(strand_bias(2, 10), 2);

        clear_variation_utils_scope();
    }

    #[test]
    fn find_conseq_uses_best_quality_to_break_count_ties() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = Sclip::default();
        sclip.base.vars_count = 4;
        sclip.nt = SortedIntMap::from([(
            0,
            SortedStringMap::from([(String::from("A"), 2), (String::from("C"), 2)]),
        )]);
        sclip.seq = SortedIntMap::from([(
            0,
            SortedStringMap::from([
                (
                    String::from("A"),
                    Variation {
                        mean_quality: 10.0,
                        ..Variation::default()
                    },
                ),
                (
                    String::from("C"),
                    Variation {
                        mean_quality: 20.0,
                        ..Variation::default()
                    },
                ),
            ]),
        )]);

        assert_eq!(find_conseq(&mut sclip, 0), "");
        assert_eq!(sclip.sequence.as_deref(), Some(""));

        clear_variation_utils_scope();
    }

    #[test]
    fn get_var_maybe_handles_missing_position() {
        let aligned: HashMap<i32, Vars> = HashMap::new();
        assert!(get_var_maybe(&aligned, 1, VarsType::Ref, VarMaybeArg::None).is_none());
    }

    #[test]
    fn find_conseq_marks_poly_t_pattern_as_used() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = Sclip::default();
        sclip.base.vars_count = 2;
        sclip.nt = (0..=12)
            .map(|index| (index, SortedStringMap::from([(String::from("T"), 2)])))
            .collect::<SortedIntMap<_>>();
        sclip.seq = (0..=12)
            .map(|index| {
                (
                    index,
                    SortedStringMap::from([(
                        String::from("T"),
                        Variation {
                            mean_quality: 40.0,
                            ..Variation::default()
                        },
                    )]),
                )
            })
            .collect::<SortedIntMap<_>>();

        assert_eq!(find_conseq(&mut sclip, 0), "TTTTTTTTTTTTT");
        assert!(sclip.used);

        clear_variation_utils_scope();
    }

    #[test]
    fn join_ref_returns_empty_when_no_positions_match() {
        let reference = HashMap::from([(10, b'A')]);
        assert_eq!(join_ref(&reference, 1, 3), "");
    }

    #[test]
    fn get_var_maybe_index_out_of_range_returns_none() {
        let vars = Vars {
            variants: vec![Variant::default()],
            ..Vars::default()
        };
        assert!(get_var_maybe_from_vars(&vars, VarsType::Var, VarMaybeArg::Index(1)).is_none());
    }

    #[test]
    fn is_low_complex_seq_treats_empty_as_low_complex() {
        assert!(is_low_complex_seq(""));
    }

    #[test]
    fn clear_variation_utils_scope_restores_defaults() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(
            Configuration {
                bias: 0.3,
                ..Configuration::default()
            },
            HashMap::from([(String::from("AAAAAA"), 1)]),
            HashMap::new(),
        );
        clear_variation_utils_scope();

        let scope = current_scope();
        assert_eq!(scope.conf.bias, Configuration::default().bias);
        assert!(scope.adaptor_forward.is_empty());
    }

    #[test]
    fn get_variation_maybe_uses_single_base_string_keys() {
        let mut entries = VariationEntries::default();
        entries.insert(String::from("C"), Variation::default());
        let hash = HashMap::from([(1, VariationMap { entries, sv: None })]);

        assert!(get_variation_maybe(&hash, 1, Some(b'C')).is_some());
    }

    #[test]
    fn weak_consensus_is_cached_as_empty_string() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());

        let mut sclip = Sclip::default();
        sclip.base.vars_count = 1;
        sclip.nt = SortedIntMap::from([(
            0,
            SortedStringMap::from([(String::from("A"), 1), (String::from("C"), 1)]),
        )]);
        sclip.seq = SortedIntMap::from([(
            0,
            SortedStringMap::from([
                (String::from("A"), Variation::default()),
                (String::from("C"), Variation::default()),
            ]),
        )]);

        assert_eq!(find_conseq(&mut sclip, 0), "");
        assert_eq!(find_conseq(&mut sclip, 0), "");

        clear_variation_utils_scope();
    }

    #[test]
    fn get_or_put_vars_returns_same_slot_for_repeated_calls() {
        let mut vars_map = HashMap::new();
        get_or_put_vars(&mut vars_map, 11).sv = String::from("SV");
        assert_eq!(get_or_put_vars(&mut vars_map, 11).sv, "SV");
    }

    #[test]
    fn adj_cnt_preserves_pstd_and_qstd_flags() {
        let mut var_to_add = Variation::default();
        adj_cnt(
            &mut var_to_add,
            &Variation {
                vars_count: 1,
                ..Variation::default()
            },
        );

        assert!(var_to_add.pstd);
        assert!(var_to_add.qstd);
    }

    #[test]
    fn get_var_maybe_ref_ignores_extra_arg() {
        let vars = Vars {
            reference_variant: Some(Variant::default()),
            ..Vars::default()
        };

        assert!(
            get_var_maybe_from_vars(&vars, VarsType::Ref, VarMaybeArg::Description("ignored"))
                .is_some()
        );
    }

    #[test]
    fn comparison_helpers_handle_negative_string_index_as_false() {
        let reference = HashMap::from([(1, b'A')]);
        assert!(!is_has_and_equals_str(&reference, 1, "A", -1));
        assert!(!is_has_and_not_equals_str(&reference, 1, "A", -1));
    }

    #[test]
    fn join_ref_for_3_lgins_can_inject_sequence_in_middle() {
        let reference = HashMap::from([(5, b'A'), (6, b'C'), (7, b'G'), (8, b'T')]);
        assert_eq!(
            join_ref_for_3_lgins(&reference, 5, 8, 2, "GGCC", "G"),
            "ACGC"
        );
    }

    #[test]
    fn join_ref_for_5_lgins_can_inject_sequence_at_end() {
        let reference = HashMap::from([(5, b'A'), (6, b'C'), (7, b'G'), (8, b'T')]);
        assert_eq!(join_ref_for_5_lgins(&reference, 5, 8, "GGCC", "G"), "ACCG");
    }

    #[test]
    fn strand_bias_uses_default_scope_without_explicit_configuration() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        clear_variation_utils_scope();
        assert_eq!(strand_bias(3, 10), 2);
    }

    #[test]
    fn get_variation_maybe_returns_none_for_non_matching_base() {
        let mut entries = VariationEntries::default();
        entries.insert(String::from("G"), Variation::default());
        let hash = HashMap::from([(4, VariationMap { entries, sv: None })]);

        assert!(get_variation_maybe(&hash, 4, Some(b'A')).is_none());
    }

    #[test]
    fn get_variation_from_seq_accepts_owned_strings() {
        let mut sclip = Sclip::default();
        get_variation_from_seq(&mut sclip, 1, String::from("T")).vars_count = 3;
        assert_eq!(
            sclip
                .seq
                .get(&1)
                .and_then(|bases| bases.get("T"))
                .map(|variation| variation.vars_count),
            Some(3)
        );
    }

    #[test]
    fn inc_cnt_handles_negative_additions() {
        let mut counts = BTreeMap::from([(1, 5)]);
        inc_cnt(&mut counts, 1, -3);
        assert_eq!(counts.get(&1), Some(&2));
    }

    #[test]
    fn configure_scope_keeps_empty_adaptor_maps_valid() {
        let _guard = TEST_SCOPE_LOCK.lock().expect("test lock poisoned");
        configure_variation_utils_scope(Configuration::default(), HashMap::new(), HashMap::new());
        let scope = current_scope();
        assert!(scope.adaptor_forward.is_empty());
        assert!(scope.adaptor_reverse.is_empty());
        clear_variation_utils_scope();
    }
}
