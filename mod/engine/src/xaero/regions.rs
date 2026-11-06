//! Xaero's World Map region files: the `.zip` container, the `region.xaero`
//! stream, and the per-pixel format (docs/XAERO-FORMATS.md).
//!
//! Reads every modern save (majors 5-8); writes major 7.8, which every
//! current build loads. Majors 0-4 (1.12-era numeric ids, NBT without
//! biome palettes) are recognized but rejected: no real modern world keeps
//! them, and the legacy slope quirks are not worth carrying.

/// The stream version we write: read by every build from 1.21.8 to 26.3.
const WRITE_MAJOR: u16 = 7;
const WRITE_MINOR: u16 = 8;

/// Bits of a pixel's params word (7.8 framing).
const P_NOT_GRASS: u32 = 1 << 0;
const P_HAS_OVERLAYS: u32 = 1 << 1;
const P_BIOME: u32 = 1 << 20;
const P_STATE_NEW: u32 = 1 << 21;
const P_BIOME_NEW: u32 = 1 << 22;
const P_TOP_HEIGHT: u32 = 1 << 24;
const P_CAVE: u32 = 1 << 7;
/// Light, bits 8-11.
const LIGHT_SHIFT: u32 = 8;

// ---------------- model ----------------

/// One block column: enough to render it and to write it back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pixel {
    /// Signed surface height (12 bits on disk).
    pub height: i16,
    /// Where the eye-level surface sits (water depth = top - height).
    pub top_height: Option<u8>,
    /// Block light just above the surface, 0-15.
    pub light: u8,
    /// The is-cave-block hint flag.
    pub cave: bool,
    /// Block state id, e.g. `minecraft:grass_block` (None = the implicit
    /// first palette entry, conventionally grass).
    pub state: Option<String>,
    /// Biome id, e.g. `minecraft:plains`.
    pub biome: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    /// 256 pixels, x outer (0..16), z inner — the stream's order.
    pub pixels: Vec<Pixel>,
    pub interp_version: u8,
    /// 0x7FFF_FFFF for a surface tile.
    pub cave_start: i32,
    pub cave_depth: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// (chunkX, chunkZ) in 0..8: block coords are
    /// region*512 + cx*64 + tileX*16 + x.
    pub at: (u8, u8),
    /// 16 tiles (tileX outer, tileZ inner); None = absent.
    pub tiles: Vec<Option<Tile>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub chunks: Vec<Chunk>,
    /// True when EOF cut the stream short (the game tolerates this).
    pub truncated: bool,
}

#[derive(Debug)]
pub enum RegionError {
    /// A legacy (major 0-4) or future (major > 8) stream.
    UnsupportedVersion {
        major: u16,
        minor: u16,
    },
    /// Not a region zip / no region.xaero entry.
    BadContainer,
    Corrupt(&'static str),
}

// ---------------- cursor ----------------

struct Rd<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], RegionError> {
        if self.pos + n > self.buf.len() {
            return Err(RegionError::Corrupt("eof"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, RegionError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, RegionError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, RegionError> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}

trait Wr {
    fn w_u8(&mut self, v: u8);
    fn w_u16(&mut self, v: u16);
    fn w_i32(&mut self, v: i32);
    fn w_bytes(&mut self, v: &[u8]);
}

impl Wr for Vec<u8> {
    fn w_u8(&mut self, v: u8) {
        self.push(v);
    }
    fn w_u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn w_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn w_bytes(&mut self, v: &[u8]) {
        self.extend_from_slice(v);
    }
}

// ---------------- NBT block states ----------------

/// Reads one inline blockstate: a Java NBT named compound (root name empty).
/// Major 8 keys it `id`/`properties`; older majors `Name`/`Properties`. Both
/// are accepted; the state's id string is returned.
fn read_state_nbt(rd: &mut Rd<'_>) -> Result<String, RegionError> {
    let tag = rd.u8()?;
    if tag != 0x0A {
        return Err(RegionError::Corrupt("state not a compound"));
    }
    let name_len = rd.u16()? as usize;
    rd.take(name_len)?; // root name, empty
    let mut id: Option<String> = None;
    loop {
        let tag = rd.u8()?;
        if tag == 0x00 {
            break;
        }
        let key_len = rd.u16()? as usize;
        let key = String::from_utf8_lossy(rd.take(key_len)?).into_owned();
        match (tag, key.as_str()) {
            (0x08, "id") | (0x08, "Name") => {
                let len = rd.u16()? as usize;
                let v = String::from_utf8_lossy(rd.take(len)?).into_owned();
                if id.is_none() {
                    id = Some(v);
                }
            }
            (0x0A, "properties") | (0x0A, "Properties") => skim_compound(rd)?,
            // Unknown key: skim whatever payload follows.
            _ => skim_payload(rd, tag)?,
        }
    }
    id.ok_or(RegionError::Corrupt("state without id"))
}

/// Skips one compound's contents (nested compounds included).
fn skim_compound(rd: &mut Rd<'_>) -> Result<(), RegionError> {
    loop {
        let tag = rd.u8()?;
        if tag == 0x00 {
            return Ok(());
        }
        let key_len = rd.u16()? as usize;
        rd.take(key_len)?;
        skim_payload(rd, tag)?;
    }
}

/// Skips one NBT payload of `tag` (the tag byte is already consumed).
fn skim_payload(rd: &mut Rd<'_>, tag: u8) -> Result<(), RegionError> {
    match tag {
        0x01 => rd.take(1).map(|_| ())?,
        0x02 => rd.take(2).map(|_| ())?,
        0x03 | 0x04 => rd.take(4).map(|_| ())?,
        0x05 | 0x06 => rd.take(8).map(|_| ())?,
        0x07 => {
            let n = rd.i32()?;
            rd.take(n.max(0) as usize)?;
        }
        0x08 => {
            let n = rd.u16()? as usize;
            rd.take(n)?;
        }
        0x09 => {
            let inner = rd.u8()?;
            let n = rd.i32()?;
            for _ in 0..n.max(0) {
                skim_payload(rd, inner)?;
            }
        }
        0x0A => skim_compound(rd)?,
        0x0B => {
            let n = rd.i32()?;
            rd.take(n.max(0) as usize * 4)?;
        }
        0x0C => {
            let n = rd.i32()?;
            rd.take(n.max(0) as usize * 8)?;
        }
        0x0D | 0x0E => {
            let n = rd.i32()?;
            for _ in 0..n.max(0) {
                skim_payload(rd, if tag == 0x0D { 0x03 } else { 0x08 })?;
            }
        }
        _ => return Err(RegionError::Corrupt("bad nbt tag")),
    }
    Ok(())
}

/// Writes an inline blockstate compound: `Name` plus optional properties.
pub fn state_nbt(name: &str, properties: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.w_u8(0x0A);
    out.w_u16(0); // empty root name
    out.w_u8(0x08);
    out.w_u16(4);
    out.w_bytes(b"Name");
    out.w_u16(name.len() as u16);
    out.w_bytes(name.as_bytes());
    if !properties.is_empty() {
        out.w_u8(0x0A);
        out.w_u16(10);
        out.w_bytes(b"Properties");
        for (k, v) in properties {
            out.w_u8(0x08);
            out.w_u16(k.len() as u16);
            out.w_bytes(k.as_bytes());
            out.w_u16(v.len() as u16);
            out.w_bytes(v.as_bytes());
        }
        out.w_u8(0x00);
    }
    out.w_u8(0x00);
    out
}

// ---------------- stream decode ----------------

/// Decodes a raw `region.xaero` stream (majors 5-8).
pub fn decode_region(stream: &[u8]) -> Result<Region, RegionError> {
    let mut rd = Rd {
        buf: stream,
        pos: 0,
    };
    if rd.u8()? != 0xFF {
        return Err(RegionError::UnsupportedVersion { major: 0, minor: 0 });
    }
    let packed = rd.i32()?;
    let major = ((packed >> 16) & 0xFFFF) as u16;
    let minor = (packed & 0xFFFF) as u16;
    if !(5..=8).contains(&major) {
        return Err(RegionError::UnsupportedVersion { major, minor });
    }
    let mut region = Region::default();
    let mut pal = Palettes::default();
    loop {
        if rd.remaining() == 0 {
            break;
        }
        let marker = match rd.u8() {
            Ok(m) => m,
            Err(_) => break,
        };
        if marker & 0x88 != 0 {
            region.truncated = true;
            break;
        }
        let mut tiles = Vec::with_capacity(16);
        for _ in 0..16 {
            let first = match rd.i32() {
                Ok(v) => v,
                Err(_) => {
                    region.truncated = true;
                    break;
                }
            };
            if first == -1 {
                tiles.push(None);
                continue;
            }
            match read_tile(&mut rd, first as u32, &mut pal) {
                Ok(t) => tiles.push(Some(t)),
                Err(_) => {
                    region.truncated = true;
                    break;
                }
            }
        }
        region.chunks.push(Chunk {
            at: (marker >> 4, marker & 7),
            tiles,
        });
        if region.truncated {
            break;
        }
    }
    Ok(region)
}

/// The region's palettes: first-occurrence order, region-local indices.
#[derive(Default)]
struct Palettes {
    states: Vec<String>,
    biomes: Vec<String>,
}

fn read_tile(rd: &mut Rd<'_>, first_params: u32, pal: &mut Palettes) -> Result<Tile, RegionError> {
    let mut pixels = Vec::with_capacity(256);
    pixels.push(read_pixel(rd, first_params, pal)?);
    for _ in 1..256 {
        let params = rd.i32()? as u32;
        pixels.push(read_pixel(rd, params, pal)?);
    }
    let interp_version = rd.u8()?;
    let cave_start = rd.i32()?;
    let cave_depth = rd.u8()?;
    Ok(Tile {
        pixels,
        interp_version,
        cave_start,
        cave_depth,
    })
}

fn read_pixel(rd: &mut Rd<'_>, params: u32, pal: &mut Palettes) -> Result<Pixel, RegionError> {
    let mut px = Pixel {
        height: unpack_height(params),
        light: ((params >> LIGHT_SHIFT) & 0xF) as u8,
        cave: params & P_CAVE != 0,
        ..Default::default()
    };
    // 1. Block state: inline NBT on first appearance, then i32 references.
    if params & P_NOT_GRASS != 0 {
        if params & P_STATE_NEW != 0 {
            let name = read_state_nbt(rd)?;
            pal.states.push(name.clone());
            px.state = Some(name);
        } else {
            let idx = rd.i32()?;
            px.state = pal
                .states
                .get(idx.unsigned_abs() as usize)
                .cloned()
                .or(Some(String::new()));
        }
    }
    // 2. Top height.
    if params & P_TOP_HEIGHT != 0 {
        px.top_height = Some(rd.u8()?);
    }
    // 3. Overlays (must be consumed to stay in sync; not kept).
    if params & P_HAS_OVERLAYS != 0 {
        let n = rd.u8()?;
        for _ in 0..n {
            read_overlay(rd, pal)?;
        }
    }
    // 4. Biome: inline UTF string on first appearance, then i32 references.
    if params & P_BIOME != 0 {
        if params & P_BIOME_NEW != 0 {
            let len = rd.u16()? as usize;
            let name = String::from_utf8_lossy(rd.take(len)?).into_owned();
            pal.biomes.push(name.clone());
            px.biome = Some(name);
        } else {
            let idx = rd.i32()?;
            px.biome = pal
                .biomes
                .get(idx.unsigned_abs() as usize)
                .cloned()
                .or(Some(String::new()));
        }
    }
    Ok(px)
}

fn read_overlay(rd: &mut Rd<'_>, pal: &mut Palettes) -> Result<(), RegionError> {
    let params = rd.i32()? as u32;
    if params & (1 << 0) != 0 {
        if params & (1 << 10) != 0 {
            let name = read_state_nbt(rd)?;
            pal.states.push(name);
        } else {
            rd.i32()?;
        }
    }
    Ok(())
}

fn unpack_height(params: u32) -> i16 {
    let raw = ((params >> 12) & 0xFF) | (((params >> 25) & 0xF) << 8);
    ((raw as i32) << 20 >> 20) as i16
}

// ---------------- stream encode (7.8) ----------------

/// Encodes a region as a 7.8 stream. Palette references become inline NBT on
/// first use and i32 indices afterwards, in first-appearance order.
pub fn encode_region(region: &Region) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(1 << 16);
    out.w_u8(0xFF);
    out.w_i32(((WRITE_MAJOR as i32) << 16) | WRITE_MINOR as i32);
    let mut states: Vec<String> = Vec::new();
    let mut biomes: Vec<String> = Vec::new();
    for chunk in &region.chunks {
        out.w_u8((chunk.at.0 << 4) | chunk.at.1);
        for tile in &chunk.tiles {
            match tile {
                None => out.w_i32(-1),
                Some(t) => encode_tile(&mut out, t, &mut states, &mut biomes),
            }
        }
    }
    out
}

fn encode_tile(out: &mut Vec<u8>, tile: &Tile, states: &mut Vec<String>, biomes: &mut Vec<String>) {
    for px in &tile.pixels {
        encode_pixel(out, px, states, biomes);
    }
    out.w_u8(tile.interp_version);
    out.w_i32(tile.cave_start);
    out.w_u8(tile.cave_depth);
}

fn encode_pixel(out: &mut Vec<u8>, px: &Pixel, states: &mut Vec<String>, biomes: &mut Vec<String>) {
    let mut params: u32 = 0;
    if px.state.is_some() {
        params |= P_NOT_GRASS;
    }
    if px.cave {
        params |= P_CAVE;
    }
    params |= u32::from(px.light & 0xF) << LIGHT_SHIFT;
    let h = (px.height as i32 as u32) & 0xFFF;
    params |= ((h & 0xFF) << 12) | ((h >> 8) << 25);
    if let Some(top) = px.top_height {
        params |= P_TOP_HEIGHT;
        let _ = top;
    }
    if px.biome.is_some() {
        params |= P_BIOME;
    }
    // First appearance: inline NBT (bit 21/22) with the raw bytes following.
    let state_new = px.state.as_ref().is_some_and(|s| !states.contains(s));
    if state_new {
        params |= P_STATE_NEW;
    }
    let biome_new = px.biome.as_ref().is_some_and(|b| !biomes.contains(b));
    if biome_new {
        params |= P_BIOME_NEW;
    }
    out.w_i32(params as i32);
    if let Some(s) = &px.state {
        if state_new {
            states.push(s.clone());
            out.w_bytes(&state_nbt(s, &[]));
        } else {
            let idx = states.iter().position(|x| x == s).unwrap_or(0);
            out.w_i32(idx as i32);
        }
    }
    if let Some(top) = px.top_height {
        out.w_u8(top);
    }
    if let Some(b) = &px.biome {
        if biome_new {
            biomes.push(b.clone());
            out.w_u16(b.len() as u16);
            out.w_bytes(b.as_bytes());
        } else {
            let idx = biomes.iter().position(|x| x == b).unwrap_or(0);
            out.w_i32(idx as i32);
        }
    }
}

// ---------------- zip container ----------------

/// Extracts the `region.xaero` payload from a region zip (one deflated entry).
pub fn unzip_region(zip: &[u8]) -> Result<Vec<u8>, RegionError> {
    let entry = zip_entries(zip).ok_or(RegionError::BadContainer)?;
    let compressed = &zip[entry.data_start..entry.data_start + entry.compressed_size];
    match entry.method {
        0 => Ok(compressed.to_vec()),
        8 => {
            let mut out = Vec::with_capacity(entry.uncompressed_size);
            let mut decoder = flate2::read::DeflateDecoder::new(compressed);
            std::io::Read::read_to_end(&mut decoder, &mut out)
                .map_err(|_| RegionError::BadContainer)?;
            Ok(out)
        }
        _ => Err(RegionError::BadContainer),
    }
}

struct ZipEntry {
    method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
    data_start: usize,
}

fn zip_entries(zip: &[u8]) -> Option<ZipEntry> {
    // End of central directory: signature PK\x05\x06, scanned from the end.
    let eocd = zip
        .windows(4)
        .rposition(|w| w == [0x50, 0x4B, 0x05, 0x06])?;
    let cd_offset = u32::from_le_bytes([
        zip[eocd + 16],
        zip[eocd + 17],
        zip[eocd + 18],
        zip[eocd + 19],
    ]) as usize;
    let entries = u16::from_le_bytes([zip[eocd + 10], zip[eocd + 11]]) as usize;
    let _ = entries;
    // First central directory entry: signature PK\x01\x02.
    let cd = zip.get(cd_offset..)?;
    if cd.len() < 46 || cd[0..4] != [0x50, 0x4B, 0x01, 0x02] {
        return None;
    }
    let method = u16::from_le_bytes([cd[10], cd[11]]);
    let compressed_size = u32::from_le_bytes([cd[20], cd[21], cd[22], cd[23]]) as usize;
    let uncompressed_size = u32::from_le_bytes([cd[24], cd[25], cd[26], cd[27]]) as usize;
    let name_len = u16::from_le_bytes([cd[28], cd[29]]) as usize;
    let extra_len = u16::from_le_bytes([cd[30], cd[31]]) as usize;
    let comment_len = u16::from_le_bytes([cd[32], cd[33]]) as usize;
    let local_offset = u32::from_le_bytes([cd[42], cd[43], cd[44], cd[45]]) as usize;
    let name = String::from_utf8_lossy(&cd[46..46 + name_len]);
    if name != "region.xaero" {
        return None;
    }
    // Local file header: signature PK\x03\x04; its own name/extra lengths
    // decide where the data starts (they may differ from the central one).
    let lh = zip.get(local_offset..)?;
    if lh.len() < 30 || lh[0..4] != [0x50, 0x4B, 0x03, 0x04] {
        return None;
    }
    let l_name = u16::from_le_bytes([lh[26], lh[27]]) as usize;
    let l_extra = u16::from_le_bytes([lh[28], lh[29]]) as usize;
    Some(ZipEntry {
        method,
        compressed_size,
        uncompressed_size,
        data_start: local_offset + 30 + l_name + l_extra,
    })
}

fn w_u16_le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn w_u32_le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Builds a single-entry region zip (deflated `region.xaero`).
pub fn zip_region(data: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(data);
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, data).expect("in-memory write");
    let compressed = encoder.finish().expect("in-memory finish");
    let name = b"region.xaero";
    let mut out = Vec::with_capacity(compressed.len() + 160);
    // Local file header.
    out.w_bytes(&[0x50, 0x4B, 0x03, 0x04]);
    w_u16_le(&mut out, 20); // version needed
    w_u16_le(&mut out, 0); // flags
    w_u16_le(&mut out, 8); // method: deflate
    w_u16_le(&mut out, 0); // time
    w_u16_le(&mut out, 0); // date
    w_u32_le(&mut out, crc);
    w_u32_le(&mut out, compressed.len() as u32);
    w_u32_le(&mut out, data.len() as u32);
    w_u16_le(&mut out, name.len() as u16);
    w_u16_le(&mut out, 0); // extra len
    out.w_bytes(name);
    out.w_bytes(&compressed);
    let local_size = out.len();
    // Central directory.
    out.w_bytes(&[0x50, 0x4B, 0x01, 0x02]);
    w_u16_le(&mut out, 20); // version made by
    w_u16_le(&mut out, 20); // version needed
    w_u16_le(&mut out, 0); // flags
    w_u16_le(&mut out, 8); // method
    w_u16_le(&mut out, 0); // time
    w_u16_le(&mut out, 0); // date
    w_u32_le(&mut out, crc);
    w_u32_le(&mut out, compressed.len() as u32);
    w_u32_le(&mut out, data.len() as u32);
    w_u16_le(&mut out, name.len() as u16);
    w_u16_le(&mut out, 0); // extra
    w_u16_le(&mut out, 0); // comment
    w_u16_le(&mut out, 0); // disk
    w_u16_le(&mut out, 0); // internal attrs
    w_u32_le(&mut out, 0); // external attrs
    w_u32_le(&mut out, 0); // local header offset (first entry, at file start)
    out.w_bytes(name);
    let cd_size = out.len() - local_size;
    // End of central directory.
    out.w_bytes(&[0x50, 0x4B, 0x05, 0x06]);
    w_u16_le(&mut out, 0); // disk
    w_u16_le(&mut out, 0); // cd disk
    w_u16_le(&mut out, 1); // entries on disk
    w_u16_le(&mut out, 1); // entries total
    w_u32_le(&mut out, cd_size as u32);
    w_u32_le(&mut out, local_size as u32);
    w_u16_le(&mut out, 0); // comment len
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_stream() {
        use super::*;
        let region = sample_region();
        let stream = encode_region(&region);
        let back = decode_region(&stream).expect("decodes");
        let t = back.chunks[0].tiles[0].as_ref().unwrap();
        for i in [0usize, 7, 14, 21] {
            println!("px{}: {:?}", i, t.pixels[i]);
        }
        let orig = region.chunks[0].tiles[0].as_ref().unwrap();
        let n = (0..256).filter(|&i| t.pixels[i] != orig.pixels[i]).count();
        println!("mismatches: {}", n);
        for i in (0..256).filter(|&i| t.pixels[i] != orig.pixels[i]).take(3) {
            println!(
                "DIFF {}: orig={:?} back={:?}",
                i, orig.pixels[i], t.pixels[i]
            );
        }
    }

    #[test]
    fn round_trips_78() {
        let region = sample_region();
        let stream = encode_region(&region);
        let back = decode_region(&stream).expect("decodes");
        assert!(!back.truncated);
        assert_eq!(back.chunks.len(), 1);
        assert_eq!(back.chunks[0].at, (0, 0));
        let tile = back.chunks[0].tiles[0].as_ref().expect("tile present");
        assert_eq!(tile.pixels.len(), 256);
        assert_eq!(
            tile.pixels[0],
            region.chunks[0].tiles[0].as_ref().unwrap().pixels[0]
        );
        assert_eq!(tile.pixels[7].height, 60);
        assert_eq!(tile.pixels[7].state.as_deref(), Some("minecraft:water"));
        assert_eq!(tile.pixels[7].biome.as_deref(), Some("minecraft:ocean"));
        // Pixel 1 is the first grass column: no stored state.
        assert_eq!(tile.pixels[1].state, None);
        assert_eq!(tile.pixels[1].biome.as_deref(), Some("minecraft:plains"));
    }

    #[test]
    fn zip_round_trips() {
        let stream = encode_region(&sample_region());
        let zipped = zip_region(&stream);
        let unzipped = unzip_region(&zipped).expect("unzips");
        assert_eq!(unzipped, stream);
    }

    #[test]
    fn negative_heights_round_trip() {
        let region = Region {
            chunks: vec![Chunk {
                at: (1, 2),
                tiles: vec![
                    Some(Tile {
                        pixels: (0..256)
                            .map(|i| Pixel {
                                height: if i % 2 == 0 { -2048 } else { 2047 },
                                ..Default::default()
                            })
                            .collect(),
                        interp_version: 1,
                        cave_start: i32::MAX,
                        cave_depth: 30,
                    }),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            }],
            truncated: false,
        };
        let back = decode_region(&encode_region(&region)).expect("decodes");
        let tile = back.chunks[0].tiles[0].as_ref().unwrap();
        assert_eq!(tile.pixels[0].height, -2048);
        assert_eq!(tile.pixels[1].height, 2047);
    }

    #[test]
    fn state_nbt_round_trip() {
        let nbt = state_nbt(
            "minecraft:oak_leaves",
            &[("distance", "7"), ("persistent", "false")],
        );
        let mut rd = Rd { buf: &nbt, pos: 0 };
        let name = read_state_nbt(&mut rd).expect("parses");
        assert_eq!(name, "minecraft:oak_leaves");
    }

    #[test]
    fn rejects_legacy_and_future() {
        // Major 6 minor 1: accepted framing-wise? majors 5-8 accepted, so use
        // major 9 to test the rejection path.
        let mut s = vec![0xFF];
        s.w_i32((9 << 16) | 8);
        s.w_u8(0);
        assert!(matches!(
            decode_region(&s),
            Err(RegionError::UnsupportedVersion { major: 9, .. })
        ));
    }
}
