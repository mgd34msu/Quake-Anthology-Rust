//! Native menu art manifest: files, dimensions, hashes, and nine-slice frames.
//!
//! Donor provenance: `src/ui/common/art-manifest.ts` in full. Dimensions and
//! hashes pin the installed assets; [`crate::ui::common::assets`] validates
//! decoded pixels against them before upload.

use qa_core::math::Vec2;

/// One installed menu image with its expected dimensions and hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuArtFile {
    /// Installed asset path.
    pub file: &'static str,
    /// Expected width in pixels.
    pub width: u32,
    /// Expected height in pixels.
    pub height: u32,
    /// Expected SHA-256 in lowercase hex.
    pub sha256: &'static str,
}

/// Nine-slice region inside one menu image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuArtRegion {
    /// Region width in pixels.
    pub width: u32,
    /// Region height in pixels.
    pub height: u32,
    /// Region corners in image UVs.
    pub uv: [Vec2; 2],
}

/// Nine-slice border insets in region pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuArtBorder {
    /// Left inset.
    pub l: u32,
    /// Top inset.
    pub t: u32,
    /// Right inset.
    pub r: u32,
    /// Bottom inset.
    pub b: u32,
}

/// One menu image plus its nine-slice frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuArtFrame {
    /// Installed asset path.
    pub file: &'static str,
    /// Expected width in pixels.
    pub width: u32,
    /// Expected height in pixels.
    pub height: u32,
    /// Expected SHA-256 in lowercase hex.
    pub sha256: &'static str,
    /// Nine-slice region.
    pub region: MenuArtRegion,
    /// Border insets in region pixels.
    pub border: MenuArtBorder,
    /// Border scale for small destinations.
    pub border_scale: f32,
}

impl MenuArtFrame {
    /// View this frame as a plain manifest file.
    #[must_use]
    pub const fn file(&self) -> MenuArtFile {
        MenuArtFile {
            file: self.file,
            width: self.width,
            height: self.height,
            sha256: self.sha256,
        }
    }
}

// Names mirror the donor manifest exports.
#[allow(non_upper_case_globals)]
/// Full-screen menu background art.
pub const menu_background: MenuArtFile = MenuArtFile {
    file: "assets/ui/menu-background.png",
    width: 1536,
    height: 1024,
    sha256: "04c4fd787c4b47f1ba27ec293c361cdf1c87406fe4d1f0005b8ba6fbe1287ffa",
};

#[allow(non_upper_case_globals)]
/// Menu panel nine-slice art.
pub const menu_panel: MenuArtFrame = MenuArtFrame {
    file: "assets/ui/menu-panel.png",
    width: 1254,
    height: 1254,
    sha256: "179d84c501384fc88443a9a415a06195270306706eb7e4ca5dc68ed06043f2a0",
    region: MenuArtRegion {
        width: 1254,
        height: 1254,
        uv: [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 1.0, y: 1.0 }],
    },
    border: MenuArtBorder {
        l: 160,
        t: 160,
        r: 160,
        b: 160,
    },
    border_scale: 0.2,
};

#[allow(non_upper_case_globals)]
/// Focused-control nine-slice art.
pub const menu_focus: MenuArtFrame = MenuArtFrame {
    file: "assets/ui/menu-focus.png",
    width: 2172,
    height: 724,
    sha256: "65b98e03aaba9f26edc36ffa0e769e2932b5b85ea994a09447edf3bfc6ba4312",
    region: MenuArtRegion {
        width: 2172,
        height: 633,
        uv: [
            Vec2 {
                x: 0.0,
                y: 68.0 / 724.0,
            },
            Vec2 {
                x: 1.0,
                y: 701.0 / 724.0,
            },
        ],
    },
    border: MenuArtBorder {
        l: 96,
        t: 96,
        r: 96,
        b: 96,
    },
    border_scale: 0.0625,
};

#[allow(non_upper_case_globals)]
/// Main-menu backdrop art.
pub const main_menu_background: MenuArtFile = MenuArtFile {
    file: "assets/ui/main-menu-background.png",
    width: 1672,
    height: 941,
    sha256: "53630f6ca2d86492e983f7e54b54ce913cbebe58df6bd847c316753ef306a718",
};

/// Every menu art file in load order.
#[must_use]
pub fn menu_art_files() -> [MenuArtFile; 4] {
    [
        menu_background,
        menu_panel.file(),
        menu_focus.file(),
        main_menu_background,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_matches_donor_values() {
        assert_eq!(menu_background.width, 1536);
        assert_eq!(menu_background.height, 1024);
        assert_eq!(
            menu_background.sha256,
            "04c4fd787c4b47f1ba27ec293c361cdf1c87406fe4d1f0005b8ba6fbe1287ffa"
        );
        assert_eq!(menu_panel.region.width, 1254);
        assert_eq!(
            menu_panel.border,
            MenuArtBorder {
                l: 160,
                t: 160,
                r: 160,
                b: 160
            }
        );
        assert_eq!(menu_panel.border_scale, 0.2);
        assert_eq!(menu_focus.region.height, 633);
        assert_eq!(menu_focus.region.uv[0].y, 68.0 / 724.0);
        assert_eq!(menu_focus.region.uv[1].y, 701.0 / 724.0);
        assert_eq!(menu_focus.border_scale, 0.0625);
        assert_eq!(main_menu_background.width, 1672);
        assert_eq!(main_menu_background.height, 941);
        assert_eq!(
            main_menu_background.sha256,
            "53630f6ca2d86492e983f7e54b54ce913cbebe58df6bd847c316753ef306a718"
        );
    }

    #[test]
    fn file_list_follows_load_order() {
        let files = menu_art_files();
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].file, "assets/ui/menu-background.png");
        assert_eq!(files[1].file, "assets/ui/menu-panel.png");
        assert_eq!(files[2].file, "assets/ui/menu-focus.png");
        assert_eq!(files[3].file, "assets/ui/main-menu-background.png");
    }
}
