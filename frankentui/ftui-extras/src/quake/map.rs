//! Quake map representation and procedural level generator.
//!
//! Provides both BSP file loading and a procedural map generator for the
//! Quake E1M1-style demo level. The procedural generator creates rooms,
//! corridors, ramps, and platforms with proper collision geometry.

use super::constants::*;

/// A face (polygon) in the map.
#[derive(Debug, Clone)]
pub struct Face {
    /// Indices into the vertex array forming this polygon.
    pub vertex_indices: Vec<usize>,
    /// Face normal.
    pub normal: [f32; 3],
    /// Distance from origin along normal.
    pub dist: f32,
    /// Surface color index (into WALL_COLORS).
    pub color_index: u8,
    /// Whether this is a sky surface.
    pub is_sky: bool,
    /// Light level (0.0-1.0).
    pub light_level: f32,
    /// Texture type for pattern generation.
    pub tex_type: TexType,
}

/// Texture type for procedural texturing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexType {
    Floor,
    Ceiling,
    Wall,
    Sky,
    Metal,
    Lava,
}

/// A wall segment for collision detection (2D).
#[derive(Debug, Clone, Copy)]
pub struct WallSeg {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub floor_z: f32,
    pub ceil_z: f32,
}

/// A room in the procedural map.
#[derive(Debug, Clone)]
pub struct Room {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub floor_z: f32,
    pub ceil_z: f32,
    pub light: f32,
}

/// The Quake map containing all geometry for rendering and collision.
#[derive(Debug, Clone)]
pub struct QuakeMap {
    /// 3D vertices.
    pub vertices: Vec<[f32; 3]>,
    /// Faces (polygons).
    pub faces: Vec<Face>,
    /// Wall segments for collision.
    pub walls: Vec<WallSeg>,
    /// Rooms for floor/ceiling height queries.
    pub rooms: Vec<Room>,
    /// Player start position.
    pub player_start: [f32; 4], // x, y, z, angle
}

impl Default for QuakeMap {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            walls: Vec::new(),
            rooms: Vec::new(),
            player_start: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

impl QuakeMap {
    /// Create an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get floor height at a 2D position by finding which room contains it.
    ///
    /// When multiple rooms overlap (e.g. a platform inside a hub), returns the
    /// highest floor — the surface the player would actually stand on.
    pub fn floor_height_at(&self, x: f32, y: f32) -> f32 {
        let mut best = None;
        for room in &self.rooms {
            if x >= room.x && x <= room.x + room.width && y >= room.y && y <= room.y + room.height {
                best = Some(match best {
                    Some(prev) => f32::max(prev, room.floor_z),
                    None => room.floor_z,
                });
            }
        }
        best.unwrap_or_else(|| {
            // Default: lowest floor
            self.rooms
                .iter()
                .map(|r| r.floor_z)
                .fold(f32::MAX, f32::min)
        })
    }

    /// Get ceiling height at a 2D position.
    ///
    /// When multiple rooms overlap, returns the lowest ceiling — the surface
    /// that would actually block the player's head.
    pub fn ceiling_height_at(&self, x: f32, y: f32) -> f32 {
        let mut best = None;
        for room in &self.rooms {
            if x >= room.x && x <= room.x + room.width && y >= room.y && y <= room.y + room.height {
                best = Some(match best {
                    Some(prev) => f32::min(prev, room.ceil_z),
                    None => room.ceil_z,
                });
            }
        }
        best.unwrap_or(1000.0)
    }

    /// Check if a point (with radius) is inside solid geometry.
    ///
    /// Only checks walls whose vertical extent overlaps the player's Z range.
    /// This prevents collisions with walls that are above or below the player
    /// (e.g. step edges of platforms the player is standing on top of).
    pub fn point_in_solid(&self, x: f32, y: f32, z: f32, radius: f32) -> bool {
        // Player occupies roughly z..z+PLAYER_HEIGHT; a wall blocks if its
        // vertical extent overlaps that range.
        let player_top = z + super::constants::PLAYER_HEIGHT;
        for wall in &self.walls {
            if wall.ceil_z <= z || wall.floor_z >= player_top {
                // Wall is entirely above or below the player — skip.
                continue;
            }
            if circle_intersects_segment(x, y, radius, wall.x1, wall.y1, wall.x2, wall.y2) {
                return true;
            }
        }
        false
    }

    /// Get the floor height the player is standing on, considering their Z.
    ///
    /// Returns the highest floor_z among overlapping rooms where the floor is
    /// at or below the player (within step-up tolerance). Falls back to
    /// the lowest matching floor if the player is below all floors (recovery).
    pub fn supportive_floor_at(&self, x: f32, y: f32, z: f32) -> f32 {
        // Use STEPSIZE as the upward tolerance so the ground check never snaps
        // the player higher than try_move's step-up limit would allow.
        // Floors below the player are always included (the <= check is trivially
        // true), and the `lowest_match` fallback handles fast-fall overshoot.
        let fall_tolerance = super::constants::STEPSIZE;
        let mut best: Option<f32> = None;
        let mut lowest_match: Option<f32> = None;
        for room in &self.rooms {
            if x >= room.x && x <= room.x + room.width && y >= room.y && y <= room.y + room.height {
                lowest_match = Some(match lowest_match {
                    Some(prev) => f32::min(prev, room.floor_z),
                    None => room.floor_z,
                });
                if room.floor_z <= z + fall_tolerance {
                    best = Some(match best {
                        Some(prev) => f32::max(prev, room.floor_z),
                        None => room.floor_z,
                    });
                }
            }
        }
        best.or(lowest_match).unwrap_or_else(|| {
            self.rooms
                .iter()
                .map(|r| r.floor_z)
                .fold(f32::MAX, f32::min)
        })
    }

    /// Get the player start position and angle.
    pub fn player_start(&self) -> (f32, f32, f32, f32) {
        (
            self.player_start[0],
            self.player_start[1],
            self.player_start[2],
            self.player_start[3],
        )
    }
}

/// Circle-segment intersection test for collision detection.
fn circle_intersects_segment(cx: f32, cy: f32, r: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-8 {
        let dist = ((cx - x1) * (cx - x1) + (cy - y1) * (cy - y1)).sqrt();
        return dist < r;
    }
    let t = ((cx - x1) * dx + (cy - y1) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);
    let nearest_x = x1 + t * dx;
    let nearest_y = y1 + t * dy;
    let dist = ((cx - nearest_x) * (cx - nearest_x) + (cy - nearest_y) * (cy - nearest_y)).sqrt();
    dist < r
}

/// Generate a procedural Quake E1M1-style level.
///
/// Creates interconnected rooms, corridors, height variations, ramps,
/// and platforms—styled after the Slipgate Complex (E1M1).
pub fn generate_e1m1() -> QuakeMap {
    let mut map = QuakeMap::new();

    // Room definitions: x, y, width, height, floor_z, ceil_z, light
    let rooms_def: Vec<(f32, f32, f32, f32, f32, f32, f32)> = vec![
        // Spawn room (large, well-lit)
        (0.0, 0.0, 256.0, 256.0, 0.0, 192.0, 0.9),
        // First corridor
        (256.0, 64.0, 192.0, 128.0, 0.0, 128.0, 0.6),
        // Central hub (big room with pillar)
        (448.0, -128.0, 384.0, 384.0, -32.0, 256.0, 0.8),
        // East corridor (elevated)
        (832.0, -32.0, 192.0, 128.0, 32.0, 160.0, 0.5),
        // East room (lava pit area)
        (1024.0, -192.0, 320.0, 320.0, -64.0, 224.0, 0.7),
        // North corridor from hub
        (544.0, 256.0, 128.0, 192.0, 0.0, 128.0, 0.4),
        // North room (dark, atmospheric)
        (480.0, 448.0, 256.0, 256.0, -16.0, 192.0, 0.3),
        // South corridor from hub
        (544.0, -320.0, 128.0, 192.0, -32.0, 128.0, 0.5),
        // South room (elevated platform)
        (480.0, -576.0, 256.0, 256.0, 48.0, 256.0, 0.7),
        // Secret passage (narrow, very dark)
        (0.0, 256.0, 64.0, 256.0, 0.0, 96.0, 0.2),
        // Secret room
        (0.0, 512.0, 128.0, 128.0, 16.0, 128.0, 0.5),
    ];

    // Create rooms
    for (i, &(rx, ry, rw, rh, fz, cz, light)) in rooms_def.iter().enumerate() {
        let room = Room {
            x: rx,
            y: ry,
            width: rw,
            height: rh,
            floor_z: fz,
            ceil_z: cz,
            light,
        };
        map.rooms.push(room);

        let color_idx = (i % WALL_COLORS.len()) as u8;
        add_room_geometry(&mut map, rx, ry, rw, rh, fz, cz, light, color_idx);
    }

    // Player start in the spawn room center
    map.player_start = [128.0, 128.0, 0.0, 0.0];

    // Add some pillars in the central hub
    add_pillar(&mut map, 576.0, 0.0, 32.0, -32.0, 192.0, 0.7, 2);
    add_pillar(&mut map, 704.0, 0.0, 32.0, -32.0, 192.0, 0.7, 2);
    add_pillar(&mut map, 576.0, 128.0, 32.0, -32.0, 192.0, 0.7, 2);
    add_pillar(&mut map, 704.0, 128.0, 32.0, -32.0, 192.0, 0.7, 2);

    // Add a raised platform in the central hub
    add_platform(&mut map, 608.0, 16.0, 96.0, 96.0, 32.0, 0.9, 4);

    map
}

/// Add a box room with floor, ceiling, and 4 walls.
#[allow(clippy::too_many_arguments)]
fn add_room_geometry(
    map: &mut QuakeMap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fz: f32,
    cz: f32,
    light: f32,
    color_idx: u8,
) {
    let base = map.vertices.len();

    // 8 corners of the box
    map.vertices.push([x, y, fz]); // 0: floor SW
    map.vertices.push([x + w, y, fz]); // 1: floor SE
    map.vertices.push([x + w, y + h, fz]); // 2: floor NE
    map.vertices.push([x, y + h, fz]); // 3: floor NW
    map.vertices.push([x, y, cz]); // 4: ceil SW
    map.vertices.push([x + w, y, cz]); // 5: ceil SE
    map.vertices.push([x + w, y + h, cz]); // 6: ceil NE
    map.vertices.push([x, y + h, cz]); // 7: ceil NW

    // Floor
    map.faces.push(Face {
        vertex_indices: vec![base, base + 1, base + 2, base + 3],
        normal: [0.0, 0.0, 1.0],
        dist: fz,
        color_index: color_idx,
        is_sky: false,
        light_level: light,
        tex_type: TexType::Floor,
    });

    // Ceiling
    map.faces.push(Face {
        vertex_indices: vec![base + 7, base + 6, base + 5, base + 4],
        normal: [0.0, 0.0, -1.0],
        dist: -cz,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.7,
        tex_type: TexType::Ceiling,
    });

    // South wall (y = ry)
    map.faces.push(Face {
        vertex_indices: vec![base, base + 4, base + 5, base + 1],
        normal: [0.0, -1.0, 0.0],
        dist: -y,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.8,
        tex_type: TexType::Wall,
    });
    map.walls.push(WallSeg {
        x1: x,
        y1: y,
        x2: x + w,
        y2: y,
        floor_z: fz,
        ceil_z: cz,
    });

    // North wall (y = ry + rh)
    map.faces.push(Face {
        vertex_indices: vec![base + 2, base + 6, base + 7, base + 3],
        normal: [0.0, 1.0, 0.0],
        dist: y + h,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.8,
        tex_type: TexType::Wall,
    });
    map.walls.push(WallSeg {
        x1: x,
        y1: y + h,
        x2: x + w,
        y2: y + h,
        floor_z: fz,
        ceil_z: cz,
    });

    // West wall (x = rx)
    map.faces.push(Face {
        vertex_indices: vec![base + 3, base + 7, base + 4, base],
        normal: [-1.0, 0.0, 0.0],
        dist: -x,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.6,
        tex_type: TexType::Wall,
    });
    map.walls.push(WallSeg {
        x1: x,
        y1: y,
        x2: x,
        y2: y + h,
        floor_z: fz,
        ceil_z: cz,
    });

    // East wall (x = rx + rw)
    map.faces.push(Face {
        vertex_indices: vec![base + 1, base + 5, base + 6, base + 2],
        normal: [1.0, 0.0, 0.0],
        dist: x + w,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.6,
        tex_type: TexType::Wall,
    });
    map.walls.push(WallSeg {
        x1: x + w,
        y1: y,
        x2: x + w,
        y2: y + h,
        floor_z: fz,
        ceil_z: cz,
    });
}

/// Add a square pillar.
#[allow(clippy::too_many_arguments)]
fn add_pillar(
    map: &mut QuakeMap,
    cx: f32,
    cy: f32,
    size: f32,
    fz: f32,
    cz: f32,
    light: f32,
    color_idx: u8,
) {
    let half = size / 2.0;
    let base = map.vertices.len();

    // 8 corners
    map.vertices.push([cx - half, cy - half, fz]);
    map.vertices.push([cx + half, cy - half, fz]);
    map.vertices.push([cx + half, cy + half, fz]);
    map.vertices.push([cx - half, cy + half, fz]);
    map.vertices.push([cx - half, cy - half, cz]);
    map.vertices.push([cx + half, cy - half, cz]);
    map.vertices.push([cx + half, cy + half, cz]);
    map.vertices.push([cx - half, cy + half, cz]);

    // 4 outer walls
    let normals = [
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
    ];
    let indices = [
        [base, base + 4, base + 5, base + 1],
        [base + 1, base + 5, base + 6, base + 2],
        [base + 2, base + 6, base + 7, base + 3],
        [base + 3, base + 7, base + 4, base],
    ];

    for (i, (norm, idx)) in normals.iter().zip(indices.iter()).enumerate() {
        map.faces.push(Face {
            vertex_indices: idx.to_vec(),
            normal: *norm,
            dist: 0.0,
            color_index: color_idx,
            is_sky: false,
            light_level: light * if i % 2 == 0 { 0.8 } else { 0.6 },
            tex_type: TexType::Metal,
        });
    }

    // Collision walls for pillar
    map.walls.push(WallSeg {
        x1: cx - half,
        y1: cy - half,
        x2: cx + half,
        y2: cy - half,
        floor_z: fz,
        ceil_z: cz,
    });
    map.walls.push(WallSeg {
        x1: cx + half,
        y1: cy - half,
        x2: cx + half,
        y2: cy + half,
        floor_z: fz,
        ceil_z: cz,
    });
    map.walls.push(WallSeg {
        x1: cx + half,
        y1: cy + half,
        x2: cx - half,
        y2: cy + half,
        floor_z: fz,
        ceil_z: cz,
    });
    map.walls.push(WallSeg {
        x1: cx - half,
        y1: cy + half,
        x2: cx - half,
        y2: cy - half,
        floor_z: fz,
        ceil_z: cz,
    });
}

/// Add a raised platform.
#[allow(clippy::too_many_arguments)]
fn add_platform(
    map: &mut QuakeMap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    height: f32,
    light: f32,
    color_idx: u8,
) {
    let base = map.vertices.len();

    // Top face vertices
    map.vertices.push([x, y, height]);
    map.vertices.push([x + w, y, height]);
    map.vertices.push([x + w, y + h, height]);
    map.vertices.push([x, y + h, height]);

    // Top face
    map.faces.push(Face {
        vertex_indices: vec![base, base + 1, base + 2, base + 3],
        normal: [0.0, 0.0, 1.0],
        dist: height,
        color_index: color_idx,
        is_sky: false,
        light_level: light,
        tex_type: TexType::Metal,
    });

    // Side faces (step edges)
    let side_base = map.vertices.len();
    map.vertices.push([x, y, 0.0]); // bottom corners
    map.vertices.push([x + w, y, 0.0]);
    map.vertices.push([x + w, y + h, 0.0]);
    map.vertices.push([x, y + h, 0.0]);

    // South step
    map.faces.push(Face {
        vertex_indices: vec![side_base, base, base + 1, side_base + 1],
        normal: [0.0, -1.0, 0.0],
        dist: -y,
        color_index: color_idx,
        is_sky: false,
        light_level: light * 0.6,
        tex_type: TexType::Metal,
    });

    // Add room entry for the platform (floor height = platform top)
    map.rooms.push(Room {
        x,
        y,
        width: w,
        height: h,
        floor_z: height,
        ceil_z: height + 160.0,
        light,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_e1m1_creates_geometry() {
        let map = generate_e1m1();
        assert!(!map.vertices.is_empty());
        assert!(!map.faces.is_empty());
        assert!(!map.walls.is_empty());
        assert!(!map.rooms.is_empty());
    }

    #[test]
    fn floor_height_in_spawn_room() {
        let map = generate_e1m1();
        // Spawn room is at (0,0) with floor_z = 0
        let h = map.floor_height_at(128.0, 128.0);
        assert!((h - 0.0).abs() < 0.01);
    }

    #[test]
    fn ceiling_in_spawn_room() {
        let map = generate_e1m1();
        let h = map.ceiling_height_at(128.0, 128.0);
        assert!((h - 192.0).abs() < 0.01);
    }

    #[test]
    fn collision_detects_walls() {
        let map = generate_e1m1();
        // Very close to the west wall of spawn room (x=0)
        assert!(map.point_in_solid(-5.0, 128.0, 10.0, 16.0));
    }

    #[test]
    fn platform_floor_height_returns_highest() {
        let map = generate_e1m1();
        // The raised platform is at (608, 16, 96x96) with floor_z=32,
        // inside the central hub which has floor_z=-32.
        // floor_height_at should return 32 (the platform), not -32 (the hub).
        let h = map.floor_height_at(656.0, 64.0);
        assert!((h - 32.0).abs() < 0.01, "expected 32.0, got {h}");
    }

    #[test]
    fn ceiling_height_returns_lowest() {
        let map = generate_e1m1();
        // Platform ceiling is 32+160=192, hub ceiling is 256.
        // Should return the lower one (192).
        let h = map.ceiling_height_at(656.0, 64.0);
        assert!((h - 192.0).abs() < 0.01, "expected 192.0, got {h}");
    }

    #[test]
    fn collision_respects_z_bounds() {
        let map = generate_e1m1();
        // West wall of spawn room has floor_z=0, ceil_z=192.
        // Player at z=10 (within wall range) should collide.
        assert!(map.point_in_solid(-5.0, 128.0, 10.0, 16.0));
        // Player far above the wall (z=500) should NOT collide.
        assert!(!map.point_in_solid(-5.0, 128.0, 500.0, 16.0));
    }

    #[test]
    fn supportive_floor_prevents_platform_teleport() {
        let map = generate_e1m1();
        // Player on hub floor (z=-32) in platform area (656, 64).
        // floor_height_at returns 32 (highest floor — the platform).
        // supportive_floor_at should return -32 (the hub), because the
        // platform at z=32 is far above the player at z=-32.
        let h = map.supportive_floor_at(656.0, 64.0, -32.0);
        assert!((h - (-32.0)).abs() < 0.01, "expected -32.0, got {h}");
    }

    #[test]
    fn supportive_floor_on_platform() {
        let map = generate_e1m1();
        // Player standing ON the platform (z=32). Both hub (-32) and
        // platform (32) are at or below z + tolerance, so returns 32.
        let h = map.supportive_floor_at(656.0, 64.0, 32.0);
        assert!((h - 32.0).abs() < 0.01, "expected 32.0, got {h}");
    }

    // ── circle_intersects_segment tests ──────────────────────────────────

    #[test]
    fn circle_misses_distant_segment() {
        assert!(!circle_intersects_segment(
            100.0, 100.0, 5.0, 0.0, 0.0, 10.0, 0.0
        ));
    }

    #[test]
    fn circle_hits_segment_center() {
        // Circle at (5, 1) with r=2, segment from (0,0) to (10,0)
        assert!(circle_intersects_segment(
            5.0, 1.0, 2.0, 0.0, 0.0, 10.0, 0.0
        ));
    }

    #[test]
    fn circle_hits_segment_endpoint() {
        // Circle centered at (0.5, 0) with r=1, segment from (1,0) to (10,0)
        assert!(circle_intersects_segment(
            0.5, 0.0, 1.0, 1.0, 0.0, 10.0, 0.0
        ));
    }

    #[test]
    fn circle_tangent_just_misses() {
        // Circle at (5, 5.1) with r=5, segment at y=0 → distance = 5.1 > 5
        assert!(!circle_intersects_segment(
            5.0, 5.1, 5.0, 0.0, 0.0, 10.0, 0.0
        ));
    }

    #[test]
    fn circle_zero_length_segment() {
        // Degenerate segment (point), circle overlaps it
        assert!(circle_intersects_segment(0.0, 0.0, 1.0, 0.5, 0.0, 0.5, 0.0));
        // Circle doesn't overlap
        assert!(!circle_intersects_segment(
            10.0, 10.0, 1.0, 0.5, 0.0, 0.5, 0.0
        ));
    }

    // ── QuakeMap direct construction tests ───────────────────────────────

    #[test]
    fn empty_map_defaults() {
        let map = QuakeMap::new();
        assert!(map.vertices.is_empty());
        assert!(map.faces.is_empty());
        assert!(map.walls.is_empty());
        assert!(map.rooms.is_empty());
        assert_eq!(map.player_start, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn floor_height_outside_all_rooms() {
        let map = generate_e1m1();
        // Very far away from any room
        let h = map.floor_height_at(99999.0, 99999.0);
        // Should return lowest floor in the map (fallback)
        let lowest = map.rooms.iter().map(|r| r.floor_z).fold(f32::MAX, f32::min);
        assert!(
            (h - lowest).abs() < 0.01,
            "outside rooms should return lowest floor, got {h}"
        );
    }

    #[test]
    fn ceiling_height_outside_all_rooms() {
        let map = generate_e1m1();
        let h = map.ceiling_height_at(99999.0, 99999.0);
        assert!(
            (h - 1000.0).abs() < 0.01,
            "outside rooms should return 1000.0, got {h}"
        );
    }

    #[test]
    fn no_collision_in_open_area() {
        let map = generate_e1m1();
        // Center of spawn room, no walls nearby
        assert!(!map.point_in_solid(128.0, 128.0, 10.0, 16.0));
    }

    #[test]
    fn player_start_tuple_matches_array() {
        let map = generate_e1m1();
        let (x, y, z, a) = map.player_start();
        assert_eq!(x, map.player_start[0]);
        assert_eq!(y, map.player_start[1]);
        assert_eq!(z, map.player_start[2]);
        assert_eq!(a, map.player_start[3]);
    }

    #[test]
    fn e1m1_has_multiple_rooms() {
        let map = generate_e1m1();
        assert!(
            map.rooms.len() >= 4,
            "expected at least 4 rooms, got {}",
            map.rooms.len()
        );
    }

    #[test]
    fn supportive_floor_outside_all_rooms() {
        let map = generate_e1m1();
        let h = map.supportive_floor_at(99999.0, 99999.0, 0.0);
        let lowest = map.rooms.iter().map(|r| r.floor_z).fold(f32::MAX, f32::min);
        assert!(
            (h - lowest).abs() < 0.01,
            "outside rooms should return lowest floor, got {h}"
        );
    }

    // ── add_room_geometry unit tests ────────────────────────────────────

    /// Helper: create a map with one room and return it.
    fn single_room_map() -> QuakeMap {
        let mut map = QuakeMap::new();
        add_room_geometry(&mut map, 0.0, 0.0, 100.0, 200.0, 0.0, 128.0, 0.8, 1);
        map
    }

    #[test]
    fn room_geometry_adds_eight_vertices() {
        let map = single_room_map();
        assert_eq!(
            map.vertices.len(),
            8,
            "box room should have 8 corner vertices"
        );
    }

    #[test]
    fn room_geometry_adds_six_faces() {
        let map = single_room_map();
        // floor + ceiling + 4 walls = 6
        assert_eq!(map.faces.len(), 6, "box room should have 6 faces");
    }

    #[test]
    fn room_geometry_adds_four_wall_segments() {
        let map = single_room_map();
        assert_eq!(
            map.walls.len(),
            4,
            "box room should have 4 collision wall segments"
        );
    }

    #[test]
    fn room_floor_face_normal_points_up() {
        let map = single_room_map();
        let floor = &map.faces[0];
        assert_eq!(floor.normal, [0.0, 0.0, 1.0]);
        assert_eq!(floor.tex_type, TexType::Floor);
    }

    #[test]
    fn room_ceiling_face_normal_points_down() {
        let map = single_room_map();
        let ceil = &map.faces[1];
        assert_eq!(ceil.normal, [0.0, 0.0, -1.0]);
        assert_eq!(ceil.tex_type, TexType::Ceiling);
    }

    #[test]
    fn room_ceiling_light_is_dimmed() {
        let map = single_room_map();
        let floor_light = map.faces[0].light_level;
        let ceil_light = map.faces[1].light_level;
        assert!(
            ceil_light < floor_light,
            "ceiling should be dimmer than floor: ceil={ceil_light} floor={floor_light}"
        );
        // Ceiling is light * 0.7
        assert!((ceil_light - 0.8 * 0.7).abs() < 0.01);
    }

    #[test]
    fn room_wall_normals_are_axis_aligned() {
        let map = single_room_map();
        // faces[2..6] are walls: south, north, west, east
        let expected_normals = [
            [0.0, -1.0, 0.0], // south
            [0.0, 1.0, 0.0],  // north
            [-1.0, 0.0, 0.0], // west
            [1.0, 0.0, 0.0],  // east
        ];
        for (i, expected) in expected_normals.iter().enumerate() {
            assert_eq!(
                map.faces[2 + i].normal,
                *expected,
                "wall {i} normal mismatch"
            );
        }
    }

    #[test]
    fn room_ns_walls_brighter_than_ew_walls() {
        let map = single_room_map();
        // South/North walls get light * 0.8, East/West get light * 0.6
        let south_light = map.faces[2].light_level;
        let west_light = map.faces[4].light_level;
        assert!(
            south_light > west_light,
            "N/S walls ({south_light}) should be brighter than E/W walls ({west_light})"
        );
        assert!((south_light - 0.8 * 0.8).abs() < 0.01);
        assert!((west_light - 0.8 * 0.6).abs() < 0.01);
    }

    #[test]
    fn room_wall_segments_match_room_bounds() {
        let map = single_room_map();
        // Room is at (0,0) size (100,200), floor=0, ceil=128
        // South wall: (0,0)→(100,0)
        let s = &map.walls[0];
        assert!((s.x1 - 0.0).abs() < 0.01);
        assert!((s.y1 - 0.0).abs() < 0.01);
        assert!((s.x2 - 100.0).abs() < 0.01);
        assert!((s.y2 - 0.0).abs() < 0.01);
        assert!((s.floor_z - 0.0).abs() < 0.01);
        assert!((s.ceil_z - 128.0).abs() < 0.01);
    }

    #[test]
    fn room_vertices_span_correct_bounds() {
        let map = single_room_map();
        // Room (0,0,100,200) floor=0 ceil=128
        // Floor corners: (0,0,0), (100,0,0), (100,200,0), (0,200,0)
        // Ceil corners: (0,0,128), (100,0,128), (100,200,128), (0,200,128)
        let xs: Vec<f32> = map.vertices.iter().map(|v| v[0]).collect();
        let ys: Vec<f32> = map.vertices.iter().map(|v| v[1]).collect();
        let zs: Vec<f32> = map.vertices.iter().map(|v| v[2]).collect();

        assert!((*xs.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 0.0).abs() < 0.01);
        assert!(
            (*xs.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 100.0).abs() < 0.01
        );
        assert!((*ys.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 0.0).abs() < 0.01);
        assert!(
            (*ys.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 200.0).abs() < 0.01
        );
        assert!((*zs.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 0.0).abs() < 0.01);
        assert!(
            (*zs.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap() - 128.0).abs() < 0.01
        );
    }

    #[test]
    fn room_all_wall_faces_are_wall_type() {
        let map = single_room_map();
        for i in 2..6 {
            assert_eq!(
                map.faces[i].tex_type,
                TexType::Wall,
                "face {i} should be Wall type"
            );
        }
    }

    // ── add_pillar unit tests ───────────────────────────────────────────

    #[test]
    fn pillar_adds_eight_vertices() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, 0.0, 100.0, 0.7, 2);
        assert_eq!(map.vertices.len(), 8);
    }

    #[test]
    fn pillar_adds_four_faces() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, 0.0, 100.0, 0.7, 2);
        assert_eq!(
            map.faces.len(),
            4,
            "pillar should have 4 wall faces (no floor/ceil)"
        );
    }

    #[test]
    fn pillar_adds_four_wall_segments() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, 0.0, 100.0, 0.7, 2);
        assert_eq!(map.walls.len(), 4);
    }

    #[test]
    fn pillar_faces_use_metal_texture() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, 0.0, 100.0, 0.7, 2);
        for face in &map.faces {
            assert_eq!(face.tex_type, TexType::Metal);
        }
    }

    #[test]
    fn pillar_walls_form_square() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 100.0, 100.0, 40.0, 0.0, 200.0, 0.7, 2);
        // half = 20.0, so corners at (80,80), (120,80), (120,120), (80,120)
        let wall_xs: Vec<f32> = map.walls.iter().flat_map(|w| [w.x1, w.x2]).collect();
        let wall_ys: Vec<f32> = map.walls.iter().flat_map(|w| [w.y1, w.y2]).collect();
        let x_min = wall_xs.iter().cloned().fold(f32::MAX, f32::min);
        let x_max = wall_xs.iter().cloned().fold(f32::MIN, f32::max);
        let y_min = wall_ys.iter().cloned().fold(f32::MAX, f32::min);
        let y_max = wall_ys.iter().cloned().fold(f32::MIN, f32::max);
        assert!((x_min - 80.0).abs() < 0.01, "pillar x_min={x_min}");
        assert!((x_max - 120.0).abs() < 0.01, "pillar x_max={x_max}");
        assert!((y_min - 80.0).abs() < 0.01, "pillar y_min={y_min}");
        assert!((y_max - 120.0).abs() < 0.01, "pillar y_max={y_max}");
    }

    #[test]
    fn pillar_wall_z_ranges_match() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, -10.0, 150.0, 0.7, 2);
        for wall in &map.walls {
            assert!((wall.floor_z - (-10.0)).abs() < 0.01);
            assert!((wall.ceil_z - 150.0).abs() < 0.01);
        }
    }

    #[test]
    fn pillar_alternating_light_levels() {
        let mut map = QuakeMap::new();
        add_pillar(&mut map, 50.0, 50.0, 20.0, 0.0, 100.0, 1.0, 2);
        // Even faces (south, north) get light * 0.8, odd (east, west) get * 0.6
        assert!((map.faces[0].light_level - 0.8).abs() < 0.01);
        assert!((map.faces[1].light_level - 0.6).abs() < 0.01);
        assert!((map.faces[2].light_level - 0.8).abs() < 0.01);
        assert!((map.faces[3].light_level - 0.6).abs() < 0.01);
    }

    // ── add_platform unit tests ─────────────────────────────────────────

    #[test]
    fn platform_adds_room_entry() {
        let mut map = QuakeMap::new();
        add_platform(&mut map, 10.0, 20.0, 50.0, 60.0, 32.0, 0.9, 4);
        assert_eq!(map.rooms.len(), 1);
        let room = &map.rooms[0];
        assert!((room.x - 10.0).abs() < 0.01);
        assert!((room.y - 20.0).abs() < 0.01);
        assert!((room.width - 50.0).abs() < 0.01);
        assert!((room.height - 60.0).abs() < 0.01);
        assert!((room.floor_z - 32.0).abs() < 0.01);
        assert!((room.ceil_z - 192.0).abs() < 0.01); // height + 160
    }

    #[test]
    fn platform_adds_top_face() {
        let mut map = QuakeMap::new();
        add_platform(&mut map, 0.0, 0.0, 64.0, 64.0, 48.0, 0.9, 4);
        // First face is the top surface
        assert!(!map.faces.is_empty());
        let top = &map.faces[0];
        assert_eq!(top.normal, [0.0, 0.0, 1.0]);
        assert!((top.dist - 48.0).abs() < 0.01);
        assert_eq!(top.tex_type, TexType::Metal);
    }

    #[test]
    fn platform_adds_south_step_face() {
        let mut map = QuakeMap::new();
        add_platform(&mut map, 0.0, 0.0, 64.0, 64.0, 48.0, 0.9, 4);
        // Second face is the south step
        assert!(map.faces.len() >= 2);
        let step = &map.faces[1];
        assert_eq!(step.normal, [0.0, -1.0, 0.0]);
        assert_eq!(step.tex_type, TexType::Metal);
    }

    #[test]
    fn platform_adds_eight_vertices() {
        let mut map = QuakeMap::new();
        add_platform(&mut map, 0.0, 0.0, 64.0, 64.0, 48.0, 0.9, 4);
        // 4 top corners + 4 bottom corners = 8
        assert_eq!(map.vertices.len(), 8);
    }

    #[test]
    fn platform_top_vertices_at_correct_height() {
        let mut map = QuakeMap::new();
        add_platform(&mut map, 10.0, 20.0, 30.0, 40.0, 50.0, 0.9, 4);
        // First 4 vertices are top corners at z=50
        for i in 0..4 {
            assert!(
                (map.vertices[i][2] - 50.0).abs() < 0.01,
                "top vertex {i} z={}",
                map.vertices[i][2]
            );
        }
        // Next 4 are bottom at z=0
        for i in 4..8 {
            assert!(
                (map.vertices[i][2] - 0.0).abs() < 0.01,
                "bottom vertex {i} z={}",
                map.vertices[i][2]
            );
        }
    }

    // ── circle_intersects_segment extra edge cases ──────────────────────

    #[test]
    fn circle_intersects_diagonal_segment() {
        // Circle at origin, segment from (-10,-10) to (10,10) passes through
        assert!(circle_intersects_segment(
            0.0, 0.0, 1.0, -10.0, -10.0, 10.0, 10.0
        ));
    }

    #[test]
    fn circle_misses_parallel_segment() {
        // Circle at (0, 5), segment along x-axis at y=0, radius=4 (gap of 1)
        assert!(!circle_intersects_segment(
            0.0, 5.0, 4.0, -100.0, 0.0, 100.0, 0.0
        ));
    }

    // ── map query edge cases ────────────────────────────────────────────

    #[test]
    fn floor_height_single_room() {
        let mut map = QuakeMap::new();
        map.rooms.push(Room {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            floor_z: 10.0,
            ceil_z: 200.0,
            light: 1.0,
        });
        assert!((map.floor_height_at(50.0, 50.0) - 10.0).abs() < 0.01);
    }

    #[test]
    fn point_in_solid_zero_radius() {
        let mut map = QuakeMap::new();
        map.walls.push(WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 0.0,
            floor_z: 0.0,
            ceil_z: 100.0,
        });
        // Zero radius: only collides if point is exactly on the segment
        // (which it won't be since distance > 0 for any off-segment point)
        assert!(!map.point_in_solid(50.0, 10.0, 50.0, 0.0));
    }

    #[test]
    fn supportive_floor_slightly_above_platform() {
        let mut map = QuakeMap::new();
        map.rooms.push(Room {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            floor_z: 0.0,
            ceil_z: 200.0,
            light: 1.0,
        });
        map.rooms.push(Room {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            floor_z: 50.0,
            ceil_z: 200.0,
            light: 1.0,
        });
        // Player at z=52, slightly above the platform (50).
        // Both floors (0 and 50) are <= z + STEPSIZE, so returns highest (50).
        let h = map.supportive_floor_at(50.0, 50.0, 52.0);
        assert!((h - 50.0).abs() < 0.01, "expected 50.0, got {h}");
    }

    #[test]
    fn floor_height_at_room_boundary() {
        let mut map = QuakeMap::new();
        map.rooms.push(Room {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            floor_z: 5.0,
            ceil_z: 200.0,
            light: 1.0,
        });
        // Exactly on the room boundary edges (inclusive)
        assert!((map.floor_height_at(0.0, 0.0) - 5.0).abs() < 0.01);
        assert!((map.floor_height_at(100.0, 100.0) - 5.0).abs() < 0.01);
        assert!((map.floor_height_at(100.0, 0.0) - 5.0).abs() < 0.01);
        assert!((map.floor_height_at(0.0, 100.0) - 5.0).abs() < 0.01);
    }

    #[test]
    fn empty_map_floor_height_fallback() {
        let map = QuakeMap::new();
        // No rooms → fold produces f32::MAX
        let h = map.floor_height_at(0.0, 0.0);
        assert_eq!(h, f32::MAX);
    }
}
