//! Xaero's Minimap `waypoints.txt` — parse and write.
//!
//! Format (docs/XAERO-FORMATS.md): UTF-8, `\n` lines. A header comment, an
//! optional `sets:` line, then `waypoint:` lines with 10-14 colon-separated
//! fields. Literal `:` inside names and initials is stored as `§§`. Unknown
//! lines are preserved verbatim so round-trips never lose data.

use crate::waypoint::{Waypoint, WaypointColor};

/// The file's header comment, exactly as Xaero writes it.
pub const HEADER: &str = "#\n#waypoint:name:initials:x:y:z:color:disabled:type:set:rotate_on_tp:tp_yaw:visibility_type:destination\n#";

/// A parsed waypoint file: the waypoints and any lines we preserve verbatim.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaypointFile {
    /// Selected set and further sets from the `sets:` line, in order.
    pub sets: Vec<String>,
    pub waypoints: Vec<StoredWaypoint>,
    /// Unknown or server_waypoint lines, preserved on write (without `\n`).
    pub extra_lines: Vec<String>,
}

/// One `waypoint:` line, keeping the raw fields Xaero's model has that ours
/// doesn't (deathpoint types, visibility, teleport data).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredWaypoint {
    pub wp: Waypoint,
    /// 1-2 char initials; empty = derive from the name on write.
    pub initials: String,
    /// None = the line had `~` (no stored Y); our Waypoint keeps pos[1] = 0.
    pub y: Option<i64>,
    /// 0 normal, 1 latest death, 2 old death.
    pub kind: i32,
    pub rotate_on_tp: bool,
    pub tp_yaw: i32,
    /// 0 local, 1 global, 2 world-map local, 3 world-map global.
    pub visibility: i32,
    /// One-off: auto-delete when reached.
    pub destination: bool,
}

impl StoredWaypoint {
    pub fn simple(wp: Waypoint) -> Self {
        Self {
            initials: wp.symbol.clone(),
            y: Some(wp.pos[1]),
            kind: 0,
            rotate_on_tp: false,
            tp_yaw: 0,
            visibility: 0,
            destination: false,
            wp,
        }
    }
}

/// Escapes a name or initials field: `:` becomes `§§`.
fn escape(s: &str) -> String {
    s.replace(":", "§§")
}

fn unescape(s: &str) -> String {
    s.replace("§§", ":")
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_i64(s: &str) -> Option<i64> {
    s.parse().ok()
}

/// Parses a whole file. Malformed `waypoint:` lines are skipped; unknown
/// lines are kept verbatim.
pub fn parse(text: &str) -> WaypointFile {
    let mut file = WaypointFile::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("sets:") {
            file.sets = rest.split(':').map(unescape).collect();
        } else if let Some(body) = line.strip_prefix("waypoint:") {
            match parse_waypoint(body) {
                Some(wp) => file.waypoints.push(wp),
                None => file.extra_lines.push(line.to_owned()),
            }
        } else if line == "#" || line.starts_with("#waypoint:") || line.trim().is_empty() {
            // Header and blanks: regenerated on write.
        } else {
            file.extra_lines.push(line.to_owned());
        }
    }
    file
}

fn parse_waypoint(body: &str) -> Option<StoredWaypoint> {
    let f: Vec<&str> = body.split(':').collect();
    if f.len() < 10 {
        return None;
    }
    let name = unescape(f[0]);
    let initials = unescape(f[1]);
    let x = parse_i64(f[2])?;
    let y = if f[3] == "~" {
        None
    } else {
        Some(parse_i64(f[3])?)
    };
    let z = parse_i64(f[4])?;
    let color_raw: i64 = parse_i64(f[5])?;
    let color_index = (color_raw.max(0) % 16) as usize;
    let disabled = parse_bool(f[6])?;
    let kind = f[7].parse::<i32>().ok()?;
    let set = f[8];
    let rotate_on_tp = if f.len() > 10 {
        parse_bool(f[9]).unwrap_or(false)
    } else {
        false
    };
    let tp_yaw = if f.len() > 11 {
        f[10].parse().unwrap_or(0)
    } else {
        0
    };
    // Field 11 was a bare bool in legacy files.
    let visibility = if f.len() > 12 {
        f[11].parse::<i32>().ok()?
    } else if f.len() > 11 {
        match parse_bool(f[11]) {
            Some(true) => 1,
            Some(false) => 0,
            None => 0,
        }
    } else {
        0
    };
    let destination = if f.len() > 13 {
        parse_bool(f[12]).unwrap_or(false)
    } else {
        false
    };
    // Deathpoints force global visibility and the destination flag.
    let (visibility, destination) = if kind == 1 || kind == 2 {
        (1, true)
    } else {
        (visibility, destination)
    };
    let color = WaypointColor::from_index(color_index);
    let wp = Waypoint {
        name,
        pos: [x, y.unwrap_or(0), z],
        color,
        enabled: !disabled,
        symbol: if initials.is_empty() {
            String::new()
        } else {
            initials
        },
        set: set.to_owned(),
    };
    Some(StoredWaypoint {
        wp,
        initials: unescape(f[1]),
        y,
        kind,
        rotate_on_tp,
        tp_yaw,
        visibility,
        destination,
    })
}

/// Writes the file. Order: sets, header, waypoints, preserved lines.
pub fn write(file: &WaypointFile) -> String {
    let mut out = String::new();
    if file.sets.len() >= 2 {
        out.push_str("sets:");
        out.push_str(
            &file
                .sets
                .iter()
                .map(|s| escape(s))
                .collect::<Vec<_>>()
                .join(":"),
        );
        out.push('\n');
    }
    out.push_str(HEADER);
    out.push('\n');
    for sw in &file.waypoints {
        out.push_str(&write_waypoint(sw));
        out.push('\n');
    }
    for line in &file.extra_lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn write_waypoint(sw: &StoredWaypoint) -> String {
    let initials = if sw.initials.is_empty() {
        default_initials(&sw.wp.name)
    } else {
        escape(&sw.initials)
    };
    let y = match sw.y {
        Some(y) => y.to_string(),
        None => "~".to_owned(),
    };
    format!(
        "waypoint:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        escape(&sw.wp.name),
        initials,
        sw.wp.pos[0],
        y,
        sw.wp.pos[2],
        sw.wp.color.index(),
        if sw.wp.enabled { "false" } else { "true" },
        sw.kind,
        sw.wp.set,
        sw.rotate_on_tp,
        sw.tp_yaw,
        sw.visibility,
        sw.destination
    )
}

/// Xaero derives initials from the name's first letters when absent.
fn default_initials(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    s.push(chars[0]);
    if chars.len() > 1 && !chars[0].is_alphanumeric() {
        s.push(chars[1]);
    } else if chars.len() > 1 && chars[1].is_uppercase() {
        s.push(chars[1]);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "#\n#waypoint:name:initials:x:y:z:color:disabled:type:set:rotate_on_tp:tp_yaw:visibility_type:destination\n#\nwaypoint:Home:H:12:64:-34:0:false:0:gui.xaero_default:false:0:0:false\nwaypoint:NoY:N:1:~:2:5:true:0:gui.xaero_default:false:0:0:false\nwaypoint:Death:D:8:70:9:0:false:1:gui.xaero_deathpoint:false:0:1:true\nserver_waypoint:somekey:false\n";

    #[test]
    fn parses_the_fixture() {
        let f = parse(SAMPLE);
        assert_eq!(f.waypoints.len(), 3);
        let home = &f.waypoints[0];
        assert_eq!(home.wp.name, "Home");
        assert_eq!(home.wp.pos, [12, 64, -34]);
        assert_eq!(home.wp.color, WaypointColor::Blue);
        assert!(home.wp.enabled);
        assert_eq!(home.initials, "H");
        assert_eq!(home.kind, 0);
        assert_eq!(home.wp.set, "gui.xaero_default");
        // The `~` Y line.
        assert_eq!(f.waypoints[1].y, None);
        assert_eq!(f.waypoints[1].wp.pos[1], 0);
        assert!(!f.waypoints[1].wp.enabled);
        // Deathpoint: kind 1, visibility forced to 1, destination true.
        assert_eq!(f.waypoints[2].kind, 1);
        assert_eq!(f.waypoints[2].visibility, 1);
        assert!(f.waypoints[2].destination);
        assert_eq!(
            f.extra_lines,
            vec!["server_waypoint:somekey:false".to_owned()]
        );
    }

    #[test]
    fn round_trips() {
        let f = parse(SAMPLE);
        let text = write(&f);
        let f2 = parse(&text);
        assert_eq!(f, f2);
    }

    #[test]
    fn escapes_colons_in_names() {
        let mut f = WaypointFile::default();
        let mut wp = Waypoint::new("a:b", 0, 0, 0, WaypointColor::Red);
        wp.symbol = "x".into();
        f.waypoints.push(StoredWaypoint::simple(wp));
        let text = write(&f);
        assert!(
            text.contains("waypoint:a§§b:x:0:0:0:5:false:0::false:0:0:false"),
            "{text}"
        );
        let f2 = parse(&text);
        assert_eq!(f2.waypoints[0].wp.name, "a:b");
    }

    #[test]
    fn skips_malformed_lines() {
        let f = parse("waypoint:only:two:fields\nwaypoint:Ok:O:1:2:3:0:false:0:gui.xaero_default:false:0:0:false\n");
        assert_eq!(f.waypoints.len(), 1);
        assert_eq!(f.waypoints[0].wp.name, "Ok");
        // The malformed line survives verbatim.
        assert!(f.extra_lines.iter().any(|l| l.starts_with("waypoint:only")));
    }

    #[test]
    fn color_modulo_16() {
        let f = parse("waypoint:X:X:0:0:0:20:false:0:gui.xaero_default:false:0:0:false\n");
        assert_eq!(f.waypoints[0].wp.color, WaypointColor::from_index(4));
    }

    #[test]
    fn legacy_visibility_bool() {
        let f = parse("waypoint:L:L:0:0:0:0:false:0:gui.xaero_default:false:0:true\n");
        assert_eq!(f.waypoints[0].visibility, 1);
    }
}
