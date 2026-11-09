//! The regions the full-resolution packages are split into: a tile goes to the first
//! region (in this order) whose box holds its middle; tiles in none go to "offshore".

pub struct Region {
    pub id: &'static str,
    pub name: &'static str,
    /// [west, south, east, north]
    pub bbox: [f64; 4],
}

pub const REGIONS: &[Region] = &[
    Region { id: "northland", name: "Northland", bbox: [172.0, -36.0, 175.0, -34.0] },
    Region { id: "auckland", name: "Auckland, Hauraki Gulf and Coromandel", bbox: [173.8, -37.4, 176.0, -36.0] },
    Region { id: "bay-of-plenty", name: "Bay of Plenty, East Cape and Gisborne", bbox: [176.0, -39.0, 179.5, -36.5] },
    Region { id: "hawkes-bay", name: "Hawke's Bay and Wairarapa", bbox: [175.5, -41.7, 179.5, -39.0] },
    Region { id: "taranaki", name: "Taranaki and Whanganui", bbox: [172.5, -40.5, 175.5, -37.4] },
    Region { id: "cook-strait", name: "Cook Strait, Wellington, Nelson and Marlborough", bbox: [171.5, -42.0, 175.5, -40.4] },
    Region { id: "canterbury", name: "Canterbury and Kaikōura", bbox: [170.5, -44.6, 175.0, -42.0] },
    Region { id: "fiordland", name: "Fiordland", bbox: [165.5, -46.6, 167.9, -44.3] },
    Region { id: "otago", name: "Otago", bbox: [169.0, -46.3, 171.8, -44.6] },
    Region { id: "southland", name: "Southland and Stewart Island", bbox: [166.5, -47.6, 169.5, -46.0] },
    Region { id: "subantarctic", name: "Subantarctic islands", bbox: [160.0, -53.0, 180.0, -47.6] },
];

pub const OFFSHORE: Region = Region { id: "offshore", name: "Offshore (shipping lanes and elsewhere)", bbox: [160.0, -53.0, 190.0, -28.0] };

/// The region a point belongs to.
pub fn region_of(lon: f64, lat: f64) -> &'static str {
    REGIONS.iter().find(|r| lon >= r.bbox[0] && lon < r.bbox[2] && lat >= r.bbox[1] && lat < r.bbox[3]).map_or(OFFSHORE.id, |r| r.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_fall_in_their_regions() {
        assert_eq!(region_of(173.28, -41.27), "cook-strait"); // Nelson
        assert_eq!(region_of(174.78, -41.29), "cook-strait"); // Wellington
        assert_eq!(region_of(174.1, -35.25), "northland"); // Bay of Islands
        assert_eq!(region_of(174.8, -36.8), "auckland");
        assert_eq!(region_of(176.2, -37.6), "bay-of-plenty"); // Tauranga
        assert_eq!(region_of(167.0, -45.4), "fiordland"); // Doubtful Sound
        assert_eq!(region_of(168.35, -46.9), "southland"); // Stewart Island
        assert_eq!(region_of(166.1, -50.6), "subantarctic"); // Auckland Islands
        assert_eq!(region_of(178.5, -45.0), "offshore");
    }
}
