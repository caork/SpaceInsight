use glam::Vec2;

/// Infinite canvas state with pan and zoom
#[derive(Debug, Clone)]
pub struct InfiniteCanvas {
    /// Pan offset in world space
    pub pan_offset: Vec2,
    /// Zoom level (0.1x to 10x)
    pub zoom_level: f32,
    /// Viewport rectangle in screen space
    pub viewport_rect: Rect,
    /// Fog radius for edge fading
    pub fog_radius: f32,
}

impl InfiniteCanvas {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            pan_offset: Vec2::ZERO,
            zoom_level: 1.0,
            viewport_rect: Rect::new(0.0, 0.0, viewport_width, viewport_height),
            fog_radius: viewport_width.max(viewport_height) * 0.6,
        }
    }

    pub fn update_viewport(&mut self, width: f32, height: f32) {
        self.viewport_rect = Rect::new(0.0, 0.0, width, height);
        self.fog_radius = width.max(height) * 0.6;
    }

    pub fn pan(&mut self, delta: Vec2) {
        self.pan_offset += delta;
    }

    pub fn zoom(&mut self, delta: f32, pivot: Vec2) {
        let old_zoom = self.zoom_level;
        self.zoom_level = (self.zoom_level + delta).clamp(0.1, 10.0);
        
        // Adjust pan to keep pivot point fixed
        let zoom_ratio = self.zoom_level / old_zoom;
        let world_pivot = self.screen_to_world(pivot);
        let new_world_pivot = (world_pivot - self.pan_offset) * zoom_ratio + self.pan_offset;
        self.pan_offset += world_pivot - new_world_pivot;
    }

    pub fn world_to_screen(&self, world_pos: Vec2) -> Vec2 {
        let center = self.viewport_rect.center();
        ((world_pos - self.pan_offset) * self.zoom_level) + center
    }

    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        let center = self.viewport_rect.center();
        (screen_pos - center) / self.zoom_level + self.pan_offset
    }

    /// Check if a world position is within the visible viewport (with margin)
    pub fn is_in_viewport(&self, world_pos: Vec2, margin: f32) -> bool {
        let screen_pos = self.world_to_screen(world_pos);
        let rect = &self.viewport_rect;
        
        screen_pos.x >= rect.x - margin
            && screen_pos.x <= rect.x + rect.width + margin
            && screen_pos.y >= rect.y - margin
            && screen_pos.y <= rect.y + rect.height + margin
    }

    /// Calculate fog alpha based on distance from viewport center
    /// Returns 0.0 (fully transparent) to 1.0 (fully visible)
    pub fn calculate_fog_alpha(&self, world_pos: Vec2) -> f32 {
        let screen_pos = self.world_to_screen(world_pos);
        let center = self.viewport_rect.center();
        let distance = (screen_pos - center).length();
        
        (1.0 - (distance / self.fog_radius)).max(0.0)
    }
}

/// Rectangle helper
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_world_to_screen() {
        let canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        // World origin should map to screen center
        let screen_pos = canvas.world_to_screen(Vec2::ZERO);
        assert!((screen_pos.x - 500.0).abs() < 0.1);
        assert!((screen_pos.y - 400.0).abs() < 0.1);
    }

    #[test]
    fn test_screen_to_world() {
        let canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        // Screen center should map to world origin
        let world_pos = canvas.screen_to_world(Vec2::new(500.0, 400.0));
        assert!(world_pos.x.abs() < 0.1);
        assert!(world_pos.y.abs() < 0.1);
    }

    #[test]
    fn test_zoom_levels() {
        let mut canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        // Test zoom in
        canvas.zoom(0.5, Vec2::new(500.0, 400.0));
        assert_eq!(canvas.zoom_level, 1.5);
        
        // Test zoom out
        canvas.zoom(-0.5, Vec2::new(500.0, 400.0));
        assert_eq!(canvas.zoom_level, 1.0);
        
        // Test clamping at min
        canvas.zoom(-5.0, Vec2::new(500.0, 400.0));
        assert_eq!(canvas.zoom_level, 0.1);
        
        // Test clamping at max
        canvas.zoom(20.0, Vec2::new(500.0, 400.0));
        assert_eq!(canvas.zoom_level, 10.0);
    }

    #[test]
    fn test_pan() {
        let mut canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        canvas.pan(Vec2::new(100.0, 50.0));
        assert_eq!(canvas.pan_offset, Vec2::new(100.0, 50.0));
        
        canvas.pan(Vec2::new(-50.0, 25.0));
        assert_eq!(canvas.pan_offset, Vec2::new(50.0, 75.0));
    }

    #[test]
    fn test_viewport_culling() {
        let canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        // Point at world origin (screen center) should be visible
        assert!(canvas.is_in_viewport(Vec2::ZERO, 0.0));
        
        // Point far away should not be visible
        assert!(!canvas.is_in_viewport(Vec2::new(10000.0, 10000.0), 0.0));
    }

    #[test]
    fn test_fog_alpha() {
        let canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        // Center should be fully visible
        let alpha_center = canvas.calculate_fog_alpha(Vec2::ZERO);
        assert!(alpha_center > 0.9);
        
        // Far edge should fade
        let alpha_edge = canvas.calculate_fog_alpha(Vec2::new(1000.0, 1000.0));
        assert!(alpha_edge < alpha_center);
    }

    #[test]
    fn test_roundtrip_conversion() {
        let canvas = InfiniteCanvas::new(1000.0, 800.0);
        
        let world_pos = Vec2::new(123.45, 678.90);
        let screen_pos = canvas.world_to_screen(world_pos);
        let back_to_world = canvas.screen_to_world(screen_pos);
        
        assert!((back_to_world.x - world_pos.x).abs() < 0.01);
        assert!((back_to_world.y - world_pos.y).abs() < 0.01);
    }
}
