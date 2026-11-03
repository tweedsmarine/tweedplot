//! The S-57 object classes TweedPlot keeps from a chart, and the table each goes in:
//! LINZ's layer names (`light_points_hydro_190k_1350k` is LIGHTS), which ENC cells
//! converted by [`crate::gpkg`] follow too (`light_points_hydro_enc_b4`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geo {
    Point,
    Line,
    Area,
}

#[derive(Debug)]
pub struct FeatureClass {
    /// S-57 object class acronym.
    pub code: &'static str,
    /// Words for people.
    pub name: &'static str,
    /// LINZ table name before `_hydro_<band>`.
    pub prefix: &'static str,
    pub geo: Geo,
}

macro_rules! fc {
    ($code:expr, $name:literal, $prefix:literal, $geo:ident) => {
        FeatureClass { code: $code, name: $name, prefix: $prefix, geo: Geo::$geo }
    };
}

/// The LINZ layers drawn on the chart. Others in the download (buildings,
/// vegetation, depth areas already handled elsewhere, ...) are ignored.
pub static CLASSES: &[FeatureClass] = &[
    fc!("LIGHTS", "Light", "light_points", Point),
    fc!("BOYLAT", "Lateral buoy", "buoy_lateral_points", Point),
    fc!("BOYCAR", "Cardinal buoy", "buoy_cardinal_points", Point),
    fc!("BOYISD", "Isolated danger buoy", "buoy_isolated_danger_points", Point),
    fc!("BOYSAW", "Safe water buoy", "buoy_safe_water_points", Point),
    fc!("BOYSPP", "Special purpose buoy", "buoy_special_purpose_general_points", Point),
    fc!("BOYINB", "Installation buoy", "buoy_installation_points", Point),
    fc!("BCNLAT", "Lateral beacon", "beacon_lateral_points", Point),
    fc!("BCNCAR", "Cardinal beacon", "beacon_cardinal_points", Point),
    fc!("BCNISD", "Isolated danger beacon", "beacon_isolated_danger_points", Point),
    fc!("BCNSAW", "Safe water beacon", "beacon_safe_water_points", Point),
    fc!("BCNSPP", "Special purpose beacon", "beacon_special_purpose_general_points", Point),
    fc!("DAYMAR", "Daymark", "daymark_points", Point),
    // not drawn on its own: its shape goes on the buoy or beacon it stands on (S-52)
    fc!("TOPMAR", "Topmark", "topmark_points", Point),
    fc!("UWTROC", "Rock", "underwater_awash_rock_points", Point),
    fc!("WRECKS", "Wreck", "wreck_points", Point),
    fc!("OBSTRN", "Obstruction", "obstruction_points", Point),
    fc!("LNDMRK", "Landmark", "landmark_points", Point),
    fc!("FOGSIG", "Fog signal", "fog_signal_points", Point),
    fc!("RTPBCN", "Racon", "radar_transponder_beacon_points", Point),
    fc!("PILBOP", "Pilot boarding place", "pilot_boarding_place_points", Point),
    fc!("ACHARE", "Anchorage", "anchorage_area_points", Point),
    fc!("ACHBRT", "Anchor berth", "anchor_berth_points", Point),
    fc!("MORFAC", "Mooring", "mooring_warping_facility_points", Point),
    fc!("PILPNT", "Pile", "pile_points", Point),
    fc!("OFSPLF", "Offshore platform", "offshore_platform_points", Point),
    fc!("MARCUL", "Marine farm", "marine_farm_culture_points", Point),
    fc!("CTNARE", "Caution", "caution_area_points", Point),
    fc!("RDOCAL", "Radio calling-in point", "radio_calling_in_point_points", Point),
    fc!("RSCSTA", "Rescue station", "rescue_station_points", Point),
    fc!("SISTAT", "Signal station", "signal_station_traffic_points", Point),
    fc!("SMCFAC", "Small craft facility", "small_craft_facility_points", Point),
    fc!("HRBFAC", "Harbour facility", "harbour_facility_points", Point),
    fc!("SEAARE", "Sea area", "sea_area_named_water_area_points", Point),
    // tidal streams: read by tidal.rs and drawn by the app (no tile symbol)
    fc!("TS_PAD", "Tidal stream panel", "tidal_stream_panel_data_points", Point),
    fc!("TS_FEB", "Tidal stream, flood or ebb", "tidal_stream_flood_ebb_points", Point),
    fc!("LNDRGN", "Land region", "land_region_points", Point),
    fc!("CBLSUB", "Submarine cable", "cable_submarine_polyline", Line),
    fc!("CBLOHD", "Overhead cable", "cable_overhead_polyline", Line),
    fc!("PIPOHD", "Overhead pipeline", "pipeline_overhead_polyline", Line),
    fc!("BRIDGE", "Bridge", "bridge_polyline", Line),
    fc!("SLCONS", "Shoreline construction", "shoreline_construction_polyline", Line),
    fc!("COALNE", "Coastline", "coastline_enc_polyline", Line),
    fc!("PONTON", "Pontoon", "pontoon_polyline", Line),
    fc!("CAUSWY", "Causeway", "causeway_polyline", Line),
    fc!("PIPSOL", "Pipeline", "pipeline_submarine_on_land_polyline", Line),
    fc!("NAVLNE", "Navigation line", "navigation_line_polyline", Line),
    fc!("RECTRC", "Recommended track", "recommended_track_polyline", Line),
    fc!("RDOCAL", "Radio calling-in line", "radio_calling_in_point_polyline", Line),
    fc!("FSHFAC", "Fishing facility", "fishing_facility_polyline", Line),
    fc!("RESARE", "Restricted area", "restricted_area_polygon", Area),
    fc!("ACHARE", "Anchorage area", "anchorage_area_polygon", Area),
    fc!("MARCUL", "Marine farm", "marine_farm_culture_polygon", Area),
    fc!("CBLARE", "Cable area", "cable_area_polygon", Area),
    fc!("CTNARE", "Caution area", "caution_area_polygon", Area),
    fc!("DMPGRD", "Dumping ground", "dumping_ground_polygon", Area),
    fc!("PRCARE", "Precautionary area", "precautionary_area_polygon", Area),
    fc!("FERYRT", "Ferry route", "ferry_route_polygons", Area),
    fc!("OBSTRN", "Obstruction area", "obstruction_polygon", Area),
    fc!("WEDKLP", "Kelp", "weed_kelp_polygon", Area),
    fc!("BRIDGE", "Bridge", "bridge_polygon", Area),
    fc!("SLCONS", "Shoreline construction", "shoreline_construction_polygon", Area),
    fc!("PONTON", "Pontoon", "pontoon_polygon", Area),
    fc!("FLODOC", "Floating dock", "floating_dock_polygon", Area),
    fc!("HULKES", "Hulk", "hulk_polygon", Area),
    fc!("HULKES", "Hulk", "hulk_polygons", Area),
    fc!("MORFAC", "Mooring facility", "mooring_warping_facility_polygon", Area),
    fc!("CAUSWY", "Causeway", "causeway_polygon", Area),
    fc!("SEAARE", "Sea area", "sea_area_named_water_area_polygon", Area),
    // ships' routeing (IMO ECDIS PS: part of the display base)
    fc!("TSSLPT", "Traffic separation scheme lane", "traffic_separation_scheme_lane_part_polygon", Area),
    fc!("TSSBND", "Traffic separation scheme boundary", "traffic_separation_scheme_boundary_polyline", Line),
    fc!("TSSCRS", "Traffic separation scheme crossing", "traffic_separation_scheme_crossing_polygon", Area),
    fc!("TSSRON", "Traffic separation scheme roundabout", "traffic_separation_scheme_roundabout_polygon", Area),
    fc!("TSEZNE", "Traffic separation zone", "traffic_separation_zone_polygon", Area),
    fc!("TSELNE", "Traffic separation line", "traffic_separation_line_polyline", Line),
    fc!("ISTZNE", "Inshore traffic zone", "inshore_traffic_zone_polygon", Area),
    fc!("FAIRWY", "Fairway", "fairway_polygon", Area),
    // any other sea object in an ENC cell: drawn with a magenta "?" (S-52 QUESMRK1) so
    // nothing in the data goes unseen; `objl` holds its S-57 class
    fc!(UNKNOWN, "Other chart object", "other_object_points", Point),
    fc!(UNKNOWN, "Other chart object", "other_object_polyline", Line),
    fc!(UNKNOWN, "Other chart object", "other_object_polygon", Area),
    // ENC objects TweedPlot's own drawing leaves out (land detail, buildings, administrative
    // limits, ...) but an ECDIS shows in their display category: the IHO rules draw them;
    // `objl` holds the S-57 class
    fc!(CHART_OBJECT, "Chart object", "chart_object_points", Point),
    fc!(CHART_OBJECT, "Chart object", "chart_object_polyline", Line),
    fc!(CHART_OBJECT, "Chart object", "chart_object_polygon", Area),
];

/// Class code of the tables for objects only the IHO rules draw ([`portrayed_only`]).
pub const CHART_OBJECT: &str = "CHTOBJ";

/// Objects TweedPlot doesn't draw itself (see [`ignored`]) that are still kept for the
/// IHO portrayal (S-52), as an ECDIS shows them: all but metadata, collections,
/// cartographic objects and the depth and land data kept elsewhere. (Tide and tidal
/// stream tables are kept: S-52 marks where they apply.)
pub fn portrayed_only(acr: &str, prim: u8) -> bool {
    ignored(acr)
        && !(acr.starts_with("M_") || acr.starts_with("C_") || acr.starts_with('$'))
        && !matches!(acr, "DEPARE" | "DRGARE" | "DEPCNT" | "SOUNDG" | "LIGHTS")
        // the land and water areas have their own tables; their points and lines (an islet,
        // a narrow river) don't
        && !(matches!(acr, "LNDARE" | "LAKARE" | "RIVERS") && prim == 3)
}

/// Class code of the catch-all tables for objects TweedPlot has no symbol for.
pub const UNKNOWN: &str = "UNKNWN";

/// Objects not drawn and not shown as unknown either: metadata, collections, land and
/// administrative features that don't concern a vessel (an ECDIS shows these, but a "?"
/// on every road and building would bury the chart). Everything else unlisted is shown.
pub fn ignored(acr: &str) -> bool {
    acr.starts_with("M_")
        || acr.starts_with("C_")
        || acr.starts_with('$')
        || acr.starts_with("T_")
        || acr.starts_with("TS_")
        || matches!(
            acr,
            "ADMARE"
                | "AIRARE"
                | "BUAARE"
                | "BUISGL"
                | "CANBNK"
                | "CHKPNT"
                | "CONVYR"
                | "CONZNE"
                | "COSARE"
                | "CTSARE"
                | "DAMCON"
                | "DYKCON"
                | "EXEZNE"
                | "FNCLNE"
                | "FORSTC"
                | "HRBARE"
                | "LNDELV"
                | "LNDRGN"
                | "MAGVAR"
                | "PRDARE"
                | "PYLONS"
                | "RAILWY"
                | "RIVBNK"
                | "ROADWY"
                | "RUNWAY"
                | "SBDARE"
                | "SILTNK"
                | "SLOTOP"
                | "SQUARE"
                | "STSLNE"
                | "TESARE"
                | "TUNNEL"
                | "VEGATN"
                | "WATFAL"
                | "ICEARE"
                | "LNDARE"
                | "LAKARE"
                | "RIVERS"
                | "CANALS"
                | "DEPARE"
                | "DRGARE"
                | "DEPCNT"
                | "SOUNDG"
                | "COALNE"
                | "LIGHTS"
                | "LNDMRK"
        )
}

/// The class of a LINZ table, by its name.
pub fn class_for(table: &str) -> Option<&'static FeatureClass> {
    let t = table.to_ascii_lowercase();
    CLASSES.iter().find(|c| t.strip_prefix(c.prefix).is_some_and(|r| r.starts_with("_hydro_")))
}
