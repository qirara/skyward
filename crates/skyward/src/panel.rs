//! The panel the display driver booted with, as the device tree describes
//! it: how bright it goes before it runs in high brightness mode.

use anyhow::{ensure, Context, Result};
use std::fs;
use std::path::Path;

/// How a panel's node in the device tree may give its normal maximum: the
/// property holding it, and the one holding the whole range on the same
/// scale. The 13 Ultra's panel gives both on a finer scale of its own that
/// spans the same light as the backlight's range; the 17 Ultra's gives them
/// on the backlight's own scale. The panels of the Xiaomi 13 and 15, of the
/// 14 and the K80 series and one of the K90's give it only as the level
/// their factory tests drive the panel to, on the backlight's own scale:
/// the 13 Ultra's panels give that level as well, and it comes to the same
/// half of their range as the normal maximum they give.
const NORMAL_MAX: [(&str, &str); 3] = [
    ("mi,normal-max-brightness-clone", "mi,max-brightness-clone"),
    ("mi,max-brightness-normal", "qcom,mdss-dsi-bl-max-level"),
    ("mi,mdss-dsi-fac-bl-max-level", "qcom,mdss-dsi-bl-max-level"),
];

/// How Samsung's panels give it instead: as the table of how much light the
/// normal levels give, a table for each revision of the panel, A first, in
/// rows of the row's index, the first and the last level it spans, the
/// level the panel itself is driven at where the node says it takes gamma
/// mode 2, and the light. A panel driven by pixel adaptive control has its
/// rows begin with an index of its own and end with a light to interpolate.
/// Its levels are the backlight's own, and the last level of the last row
/// is the normal maximum, the table of the high brightness mode taking
/// over above it.
const SAMSUNG_TABLE: &str = "samsung,candela_map_table_rev";
const SAMSUNG_PAC_TABLE: &str = "samsung,pac_candela_map_table_rev";

/// The raw backlight level, out of `max`, at the panel's normal maximum.
/// Above it the panel runs in high brightness mode, which the stock system
/// keeps for strong sunlight and its slider does not reach. The panel the
/// display driver booted with gives that maximum in the device tree.
#[cfg(target_os = "android")]
pub fn normal_max(max: u32) -> Result<u32> {
    let param = "/sys/module/msm_drm/parameters/dsi_display0";
    let boot = fs::read_to_string(param).with_context(|| format!("read {param}"))?;
    // The panel's node, then its configuration after a colon, as in
    // "qcom,mdss_dsi_m1_42_02_0a_dsc_cmd:"; the driver takes no name
    // without the colon either.
    let (name, _) = boot.trim().split_once(':').with_context(|| format!("{param} names no panel: {boot:?}"))?;
    let soc = Path::new("/proc/device-tree/soc");
    let node = fs::read_dir(soc)
        .with_context(|| format!("read {}", soc.display()))?
        .map(|e| e.map(|e| e.path().join(name)))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .find(|p| p.is_dir())
        .ok_or_else(|| anyhow::anyhow!("panel {name} is not in the device tree"))?;
    normal_max_of(&node, name, max)
}

/// The raw backlight level, out of `max`, at the normal maximum of the
/// panel `name` whose node in the device tree is `node`.
fn normal_max_of(node: &Path, name: &str, max: u32) -> Result<u32> {
    let cells = |prop: &str| -> Result<Vec<u32>> {
        let path = node.join(prop);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let (cells, rest) = bytes.as_chunks::<4>();
        ensure!(!cells.is_empty() && rest.is_empty(), "{} is not a list of cells", path.display());
        Ok(cells.iter().map(|c| u32::from_be_bytes(*c)).collect())
    };
    let cell = |prop: &str| -> Result<u32> {
        match cells(prop)?[..] {
            [cell] => Ok(cell),
            _ => anyhow::bail!("{} is not one cell", node.join(prop).display()),
        }
    };
    if let Some((normal, full)) = NORMAL_MAX.iter().find(|(normal, _)| node.join(normal).exists()) {
        let (normal, full) = (cell(normal)?, cell(full)?);
        ensure!(normal > 0 && normal <= full, "panel {name}: normal maximum {normal} out of {full}");
        return Ok((f64::from(max) * f64::from(normal) / f64::from(full)).round() as u32);
    }
    // A property that holds no value, as these two, is there or not.
    let (prefix, columns, last_level) = match (node.join("samsung,support_pac").exists(), node.join("samsung,support_gamma_mode2").exists()) {
        (true, _) => (SAMSUNG_PAC_TABLE, 6, 3),
        (false, true) => (SAMSUNG_TABLE, 5, 2),
        (false, false) => (SAMSUNG_TABLE, 4, 2),
    };
    let table = ('A'..='T').map(|revision| format!("{prefix}{revision}")).find(|table| node.join(table).exists()).with_context(|| {
        format!("panel {name} gives its normal maximum as none of {NORMAL_MAX:?}, nor in a table {prefix}A to {prefix}T as Samsung's do")
    })?;
    let rows = cells(&table)?;
    ensure!(rows.len() % columns == 0, "panel {name}: {table} is not rows of {columns}");
    let normal = rows.chunks_exact(columns).map(|row| row[last_level]).next_back().unwrap_or_default();
    ensure!(normal > 0, "panel {name}: {table} ends at level {normal}");
    // A backlight that does not reach the high brightness mode at all has
    // its own maximum as the normal one.
    Ok(normal.min(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node of the device tree holding each property as a file, as
    /// /proc/device-tree has them, its cells big-endian.
    fn node(props: &[(&str, &[u32])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, cells) in props {
            let bytes: Vec<u8> = cells.iter().flat_map(|c| c.to_be_bytes()).collect();
            fs::write(dir.path().join(name), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn xiaomi_s_panels_give_it_on_a_scale_of_their_own() {
        let n = node(&[("mi,max-brightness-normal", &[2047]), ("qcom,mdss-dsi-bl-max-level", &[4095])]);
        assert_eq!(normal_max_of(n.path(), "p", 4095).unwrap(), 2047);
        let n = node(&[("mi,normal-max-brightness-clone", &[8191]), ("mi,max-brightness-clone", &[16383])]);
        assert_eq!(normal_max_of(n.path(), "p", 2047).unwrap(), 1023, "scaled to the backlight's range");
    }

    #[test]
    fn samsung_s_panels_end_their_table_of_normal_levels_at_it() {
        // The S21 Ultra's S6E3HAD: normal levels up to 255 at 500 nits, and
        // the backlight's range up to 561 in high brightness mode.
        let rows: &[u32] = &[0, 0, 3, 3, 2, 1, 4, 5, 5, 3, 74, 254, 255, 2047, 500];
        let n = node(&[("samsung,support_gamma_mode2", &[]), ("samsung,candela_map_table_revA", rows)]);
        assert_eq!(normal_max_of(n.path(), "ss_dsi_panel_S6E3HAD_AMB681XV01_WQHD", 561).unwrap(), 255);
        assert_eq!(normal_max_of(n.path(), "p", 200).unwrap(), 200, "a backlight short of it stops at its own maximum");
        let four: &[u32] = &[0, 0, 3, 2, 1, 4, 255, 420];
        assert_eq!(normal_max_of(node(&[("samsung,candela_map_table_revA", four)]).path(), "p", 365).unwrap(), 255, "without gamma mode 2");
        let pac: &[u32] = &[0, 0, 0, 3, 2, 2, 9, 1, 4, 255, 420, 420];
        let n = node(&[("samsung,support_pac", &[]), ("samsung,pac_candela_map_table_revA", pac)]);
        assert_eq!(normal_max_of(n.path(), "p", 365).unwrap(), 255, "rows of six under pixel adaptive control");
        let later = node(&[("samsung,candela_map_table_revC", four)]);
        assert_eq!(normal_max_of(later.path(), "p", 365).unwrap(), 255, "the first revision the node has a table for");
    }

    #[test]
    fn a_panel_that_gives_it_no_way_known_or_a_table_out_of_shape_is_an_error() {
        let e = normal_max_of(node(&[]).path(), "p", 255).unwrap_err();
        assert!(format!("{e:#}").contains("nor in a table"), "{e:#}");
        let ragged = node(&[("samsung,candela_map_table_revA", &[0, 0, 3, 2, 1])]);
        assert!(normal_max_of(ragged.path(), "p", 255).is_err(), "rows of four with one left over");
        let dark = node(&[("samsung,candela_map_table_revA", &[0, 0, 0, 2])]);
        assert!(normal_max_of(dark.path(), "p", 255).is_err(), "a table ending at level 0");
    }
}
