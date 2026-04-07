use std::hash::{Hash, Hasher};

// Java: Region L9-L106
#[derive(Clone, Debug)]
pub struct Region {
    pub chr: String,
    pub start: i32,
    pub end: i32,
    pub gene: String,
    pub insert_start: i32,
    pub insert_end: i32,
}

impl Region {
    pub fn new(chr: impl Into<String>, start: i32, end: i32, gene: impl Into<String>) -> Self {
        Self::new_with_insert_range(chr, start, end, gene, 0, 0)
    }

    pub fn new_with_insert_range(
        chr: impl Into<String>,
        start: i32,
        end: i32,
        gene: impl Into<String>,
        insert_start: i32,
        insert_end: i32,
    ) -> Self {
        Self {
            chr: chr.into(),
            start,
            end,
            gene: gene.into(),
            insert_start,
            insert_end,
        }
    }

    pub fn new_modified_region(region: &Self, changed_start: i32, changed_end: i32) -> Self {
        Self::new_with_insert_range(
            region.chr.clone(),
            changed_start,
            changed_end,
            region.gene.clone(),
            region.insert_start,
            region.insert_end,
        )
    }

    pub fn print_region(&self) -> String {
        format!("{}:{}-{}", self.chr, self.start, self.end)
    }
}

impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        self.chr == other.chr && self.start == other.start && self.end == other.end
    }
}

impl Eq for Region {}

impl Hash for Region {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.chr.hash(state);
        self.start.hash(state);
        self.end.hash(state);
    }
}

// Java: Side L6-L11
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Three,
    Five,
    Unknown,
}

impl Side {
    pub fn from_int(side: i32) -> Self {
        match side {
            3 => Self::Three,
            5 => Self::Five,
            _ => Self::Unknown,
        }
    }
}

// TODO(S09): replace this placeholder with the full Sclip port from
// VarDictJava/src/main/java/com/astrazeneca/vardict/variations/Sclip.java.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sclip;

// Java: SortPositionSclip L8-L24
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SortPositionSclip {
    pub position: i32,
    pub soft_clip: Sclip,
    pub count: i32,
}

impl SortPositionSclip {
    pub fn new(position: i32, soft_clip: Sclip, count: i32) -> Self {
        Self {
            position,
            soft_clip,
            count,
        }
    }
}

// Java: BaseInsertion L7-L24
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BaseInsertion {
    pub base_insert: Option<i32>,
    pub insertion_sequence: String,
    pub base_insert2: Option<i32>,
}

impl BaseInsertion {
    pub fn new(base_insert: i32, insertion_sequence: impl Into<String>, base_insert2: i32) -> Self {
        Self {
            base_insert: Some(base_insert),
            insertion_sequence: insertion_sequence.into(),
            base_insert2: Some(base_insert2),
        }
    }
}

// Java: Match L6-L13
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Match {
    pub base_position: i32,
    pub matched_sequence: String,
}

impl Match {
    pub fn new(base_position: i32, matched_sequence: impl Into<String>) -> Self {
        Self {
            base_position,
            matched_sequence: matched_sequence.into(),
        }
    }
}

// Java: Match35 L6-L23
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Match35 {
    pub matched_5_end: i32,
    pub matched_3_end: i32,
    pub max_matched_length: i32,
}

impl Match35 {
    pub fn new(matched_5_end: i32, matched_3_end: i32, max_matched_length: i32) -> Self {
        Self {
            matched_5_end,
            matched_3_end,
            max_matched_length,
        }
    }
}

// Java: ModifiedCigar L3-L14
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModifiedCigar {
    pub position: i32,
    pub cigar: String,
    pub query_sequence: String,
    pub query_quality: String,
}

impl ModifiedCigar {
    pub fn new(
        position: i32,
        cigar: impl Into<String>,
        query_sequence: impl Into<String>,
        query_quality: impl Into<String>,
    ) -> Self {
        Self {
            position,
            cigar: cigar.into(),
            query_sequence: query_sequence.into(),
            query_quality: query_quality.into(),
        }
    }
}

// Java: CurrentSegment L6-L15
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CurrentSegment {
    pub chr: String,
    pub start: i32,
    pub end: i32,
}

impl CurrentSegment {
    pub fn new(chr: impl Into<String>, start: i32, end: i32) -> Self {
        Self {
            chr: chr.into(),
            start,
            end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_region(region: &Region) -> u64 {
        let mut hasher = DefaultHasher::new();
        region.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn region_new_sets_default_insert_range() {
        let region = Region::new("chr1", 10, 20, "GENE1");

        assert_eq!(region.chr, "chr1");
        assert_eq!(region.start, 10);
        assert_eq!(region.end, 20);
        assert_eq!(region.gene, "GENE1");
        assert_eq!(region.insert_start, 0);
        assert_eq!(region.insert_end, 0);
        assert_eq!(region.print_region(), "chr1:10-20");
    }

    #[test]
    fn region_modified_region_preserves_metadata() {
        let region = Region::new_with_insert_range("chr2", 100, 200, "GENE2", 110, 190);
        let modified = Region::new_modified_region(&region, 90, 210);

        assert_eq!(modified.chr, "chr2");
        assert_eq!(modified.start, 90);
        assert_eq!(modified.end, 210);
        assert_eq!(modified.gene, "GENE2");
        assert_eq!(modified.insert_start, 110);
        assert_eq!(modified.insert_end, 190);
    }

    #[test]
    fn region_equality_and_hash_only_use_coordinates() {
        let left = Region::new_with_insert_range("chr3", 7, 9, "GENE_A", 1, 2);
        let right = Region::new_with_insert_range("chr3", 7, 9, "GENE_B", 10, 20);

        assert_eq!(left, right);
        assert_eq!(hash_region(&left), hash_region(&right));
    }

    #[test]
    fn side_from_int_maps_java_values() {
        assert_eq!(Side::from_int(3), Side::Three);
        assert_eq!(Side::from_int(5), Side::Five);
        assert_eq!(Side::from_int(0), Side::Unknown);
        assert_eq!(Side::from_int(-1), Side::Unknown);
    }

    #[test]
    fn small_data_types_construct() {
        let sort_position = SortPositionSclip::new(12, Sclip, 3);
        let insertion = BaseInsertion::new(8, "AC", 6);
        let matched = Match::new(15, "TT");
        let matched_ends = Match35::new(5, 9, 4);
        let modified_cigar = ModifiedCigar::new(20, "10M1I5M", "ACGT", "!!!!");
        let segment = CurrentSegment::new("chr7", 30, 40);

        assert_eq!(sort_position.position, 12);
        assert_eq!(sort_position.count, 3);
        assert_eq!(insertion.base_insert, Some(8));
        assert_eq!(insertion.insertion_sequence, "AC");
        assert_eq!(insertion.base_insert2, Some(6));
        assert_eq!(matched.base_position, 15);
        assert_eq!(matched.matched_sequence, "TT");
        assert_eq!(matched_ends.matched_5_end, 5);
        assert_eq!(matched_ends.matched_3_end, 9);
        assert_eq!(matched_ends.max_matched_length, 4);
        assert_eq!(modified_cigar.position, 20);
        assert_eq!(modified_cigar.cigar, "10M1I5M");
        assert_eq!(modified_cigar.query_sequence, "ACGT");
        assert_eq!(modified_cigar.query_quality, "!!!!");
        assert_eq!(segment.chr, "chr7");
        assert_eq!(segment.start, 30);
        assert_eq!(segment.end, 40);
    }
}
