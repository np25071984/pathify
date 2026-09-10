//! Web Mercator, the projection rendered maps are actually drawn in.
//!
//! `pathify-tui` projects equirectangularly — longitude scaled by the cosine of
//! a reference latitude — which is right for a picture of a trace on its own,
//! where nothing else has to line up. It is wrong here. A downloaded map tile
//! or export is Web Mercator, and drawing an equirectangular track over it
//! shears the line away from the roads it followed, worse the further from the
//! reference latitude. So this is a second projection rather than a reuse of
//! the first.

use crate::{Error, Result};

/// WGS84 semi-major axis, the radius Web Mercator is defined against.
const EARTH_RADIUS_M: f64 = 6_378_137.0;

/// Web Mercator's northing goes to infinity at the poles, so the projection is
/// conventionally cut off here — the latitude whose normalized y is exactly 0
/// or 1. Every tile server in existence uses this limit.
pub const MAX_LATITUDE: f64 = 85.051_128_779_806_59;

/// Project to normalized Web Mercator: 0..1 west to east, and 0..1 *north to
/// south*, because that is the direction image rows run.
pub fn normalized(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-MAX_LATITUDE, MAX_LATITUDE);
    let x = (lon + 180.0) / 360.0;
    let phi = lat.to_radians();
    let y = 0.5 - (phi.tan() + 1.0 / phi.cos()).ln() / (2.0 * std::f64::consts::PI);
    (x, y)
}

/// A geographic bounding box mapped onto an image of a known pixel size.
///
/// The image is assumed to cover the box exactly, in Web Mercator, which is
/// what a `bbox=` export gives you. Pathify cannot check that assumption — a
/// PNG carries no geographic metadata — so the box has to be told to it, and
/// [`Basemap::aspect_skew`] exists to catch the common case of being told the
/// wrong one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Basemap {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    width_px: f64,
    height_px: f64,
    center_lat: f64,
}

impl Basemap {
    /// `bbox` is `[min_lon, min_lat, max_lon, max_lat]`, matching
    /// `Bounds::bbox` and every map service's `bbox=` parameter.
    ///
    /// A box with no extent on either axis is rejected rather than nudged into
    /// shape: it cannot describe an image, and every scale derived from it
    /// would be an infinity that silently poisons the pixel coordinates.
    pub fn new(bbox: [f64; 4], width_px: u32, height_px: u32) -> Result<Self> {
        let [min_lon, min_lat, max_lon, max_lat] = bbox;
        if width_px == 0 || height_px == 0 {
            return Err(Error::EmptyImage);
        }

        let (left, bottom) = normalized(min_lat, min_lon);
        let (right, top) = normalized(max_lat, max_lon);
        if !(right - left).is_finite()
            || !(bottom - top).is_finite()
            || right <= left
            || bottom <= top
        {
            return Err(Error::DegenerateBbox);
        }

        Ok(Self {
            left,
            top,
            right,
            bottom,
            width_px: f64::from(width_px),
            height_px: f64::from(height_px),
            center_lat: (min_lat + max_lat) / 2.0,
        })
    }

    /// Pixel coordinates of a geographic point, with the origin at the image's
    /// top-left corner. Not clamped: a point outside the box lands outside the
    /// image, which is what lets a line into or out of frame be clipped
    /// correctly rather than dragged to the edge.
    pub fn to_pixel(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = normalized(lat, lon);
        (
            (x - self.left) / (self.right - self.left) * self.width_px,
            (y - self.top) / (self.bottom - self.top) * self.height_px,
        )
    }

    /// Ground meters per pixel at the box's centre latitude.
    ///
    /// One number covers both axes: Mercator is conformal, so at any given
    /// point it scales x and y alike. It varies with latitude across a large
    /// box, which is why this is the *centre* — for a box the size anyone
    /// renders a ride on, the variation is far below a pixel.
    pub fn meters_per_pixel(&self) -> f64 {
        // A full turn of normalized x is the equator's circumference; at
        // latitude φ the parallel is shorter by cos φ.
        let ground_width_m = (self.right - self.left)
            * 2.0
            * std::f64::consts::PI
            * EARTH_RADIUS_M
            * self.center_lat.to_radians().cos();
        ground_width_m / self.width_px
    }

    /// How far the image's aspect ratio departs from the box's, as a ratio
    /// where 1.0 is a perfect match.
    ///
    /// Worth reporting and not worth failing over. A basemap a few percent off
    /// still produces a useful picture; one that is wildly off is almost always
    /// an image downloaded for a different box than the one passed, and saying
    /// so beats handing back a plausible-looking lie.
    pub fn aspect_skew(&self) -> f64 {
        let bbox_aspect = (self.right - self.left) / (self.bottom - self.top);
        let image_aspect = self.width_px / self.height_px;
        image_aspect / bbox_aspect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values, not a round trip: a symmetric projection test passes
    /// happily with the axes transposed.
    #[test]
    fn null_island_is_the_middle_of_the_world() {
        let (x, y) = normalized(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-12, "{x}");
        assert!((y - 0.5).abs() < 1e-12, "{y}");
    }

    #[test]
    fn the_projection_limits_are_the_corners() {
        let (x, y) = normalized(MAX_LATITUDE, -180.0);
        assert!(x.abs() < 1e-12, "{x}");
        assert!(y.abs() < 1e-9, "{y}");

        let (x, y) = normalized(-MAX_LATITUDE, 180.0);
        assert!((x - 1.0).abs() < 1e-12, "{x}");
        assert!((y - 1.0).abs() < 1e-9, "{y}");
    }

    /// Beyond the cutoff the projection would run away to infinity, so it is
    /// clamped rather than allowed to produce a coordinate no image can hold.
    #[test]
    fn the_poles_are_clamped_rather_than_infinite() {
        let (_, y) = normalized(90.0, 0.0);
        assert!(y.is_finite() && y.abs() < 1e-9, "{y}");
        let (_, y) = normalized(-90.0, 0.0);
        assert!(y.is_finite() && (y - 1.0).abs() < 1e-9, "{y}");
    }

    /// The load-bearing assertion in this module: published EPSG:3857
    /// northings, which are what the map being drawn on was itself rendered
    /// from. These are external reference values, not this code's own output
    /// fed back in — asserting a slippy-map tile index would prove nothing,
    /// since the tile index *is* this formula.
    #[test]
    fn agrees_with_published_epsg_3857_northings() {
        let northing = |lat: f64| {
            let (_, y) = normalized(lat, 0.0);
            (0.5 - y) * 2.0 * std::f64::consts::PI * EARTH_RADIUS_M
        };

        assert!(
            (northing(45.0) - 5_621_521.49).abs() < 0.01,
            "{}",
            northing(45.0)
        );
        assert!(
            (northing(60.0) - 8_399_737.89).abs() < 0.01,
            "{}",
            northing(60.0)
        );
        // The southern hemisphere is the same distance the other way.
        assert!(
            (northing(-45.0) + 5_621_521.49).abs() < 0.01,
            "{}",
            northing(-45.0)
        );
        // And the cutoff latitude is half the equator's circumference, the
        // constant every tile server's bounds are quoted in.
        assert!(
            (northing(MAX_LATITUDE) - 20_037_508.34).abs() < 0.01,
            "{}",
            northing(MAX_LATITUDE)
        );
    }

    fn seattle_basemap() -> Basemap {
        Basemap::new([-122.3056, 47.6535, -122.2745, 47.6651], 800, 600).unwrap()
    }

    #[test]
    fn the_bbox_corners_land_on_the_image_corners() {
        let map = seattle_basemap();

        let (x, y) = map.to_pixel(47.6651, -122.3056);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6, "top-left was ({x}, {y})");

        let (x, y) = map.to_pixel(47.6535, -122.2745);
        assert!(
            (x - 800.0).abs() < 1e-6 && (y - 600.0).abs() < 1e-6,
            "bottom-right was ({x}, {y})"
        );
    }

    /// Northward is up. Transposing or flipping the y axis is the single
    /// easiest mistake to make here and the hardest to spot in a finished
    /// image, so it gets its own test.
    #[test]
    fn latitude_increases_upward_and_longitude_rightward() {
        let map = seattle_basemap();
        let (mid_x, mid_y) = map.to_pixel(47.6593, -122.29005);
        let (north_x, north_y) = map.to_pixel(47.6640, -122.29005);
        let (_, east_y) = map.to_pixel(47.6593, -122.2800);
        let (east_x, _) = map.to_pixel(47.6593, -122.2800);

        assert!(north_y < mid_y, "north should be nearer the top row");
        assert!((north_x - mid_x).abs() < 1e-6, "north should not move x");
        assert!(east_x > mid_x, "east should be nearer the right column");
        assert!((east_y - mid_y).abs() < 1e-6, "east should not move y");
    }

    /// Seattle's latitude shrinks a degree of longitude to about 67 km, so a
    /// 0.0311-degree-wide box across 800 px is a little under 3 m per pixel.
    #[test]
    fn meters_per_pixel_matches_the_ground_distance() {
        let mpp = seattle_basemap().meters_per_pixel();
        assert!((mpp - 2.91).abs() < 0.05, "{mpp} m/px");
    }

    #[test]
    fn a_matching_image_has_no_aspect_skew() {
        // The box is 0.0311 deg of longitude by 0.0116 of latitude, which in
        // Mercator is very nearly 1.806:1 — so 723 x 400 is the shape of image
        // it should be downloaded as.
        let map = Basemap::new([-122.3056, 47.6535, -122.2745, 47.6651], 723, 400).unwrap();
        assert!(
            (map.aspect_skew() - 1.0).abs() < 0.01,
            "{}",
            map.aspect_skew()
        );

        // Half the width it should be, and the skew says so.
        let squashed = Basemap::new([-122.3056, 47.6535, -122.2745, 47.6651], 361, 400).unwrap();
        assert!(
            (squashed.aspect_skew() - 0.5).abs() < 0.01,
            "{}",
            squashed.aspect_skew()
        );
    }

    /// A single-point trace has bounds with no extent. Accepting it would make
    /// every pixel coordinate an infinity or a NaN, and the failure would
    /// surface as a blank image rather than as an error.
    #[test]
    fn a_box_with_no_extent_is_rejected() {
        let point = [-122.3056, 47.6535, -122.3056, 47.6535];
        assert!(matches!(
            Basemap::new(point, 800, 600),
            Err(Error::DegenerateBbox)
        ));

        let no_height = [-122.3056, 47.6535, -122.2745, 47.6535];
        assert!(matches!(
            Basemap::new(no_height, 800, 600),
            Err(Error::DegenerateBbox)
        ));

        let inverted = [-122.2745, 47.6535, -122.3056, 47.6651];
        assert!(matches!(
            Basemap::new(inverted, 800, 600),
            Err(Error::DegenerateBbox)
        ));
    }

    #[test]
    fn an_image_with_no_pixels_is_rejected() {
        let bbox = [-122.3056, 47.6535, -122.2745, 47.6651];
        assert!(matches!(Basemap::new(bbox, 0, 600), Err(Error::EmptyImage)));
    }
}
