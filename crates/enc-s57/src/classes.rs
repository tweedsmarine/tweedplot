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
    ($code:literal, $name:literal, $prefix:literal, $geo:ident) => {
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
];

/// The class of a LINZ table, by its name.
pub fn class_for(table: &str) -> Option<&'static FeatureClass> {
    let t = table.to_ascii_lowercase();
    CLASSES.iter().find(|c| t.strip_prefix(c.prefix).is_some_and(|r| r.starts_with("_hydro_")))
}
