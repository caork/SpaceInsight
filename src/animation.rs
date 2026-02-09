/// Animated rectangle with position, size, and opacity
#[derive(Debug, Clone, Copy)]
pub struct AnimRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub opacity: f32,
}

/// Per-rect animation state with spring physics
#[derive(Debug, Clone)]
pub struct RectAnimState {
    pub current: AnimRect,
    pub target: AnimRect,
    velocity_x: f32,
    velocity_y: f32,
    velocity_w: f32,
    velocity_h: f32,
    reveal_delay: f32,
    pub is_revealed: bool,
    pub is_settled: bool,
    pub index: usize,
}

/// Performance tier based on item count
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationTier {
    /// < 300 items: full animation (6 batches, spring settling)
    Full,
    /// 300-1000 items: degraded (2 batches, faster settling)
    Degraded,
    /// > 1000 items: no animation (instant snap)
    Skip,
}

impl AnimationTier {
    pub fn from_item_count(count: usize) -> Self {
        if count > 1000 {
            Self::Skip
        } else if count > 300 {
            Self::Degraded
        } else {
            Self::Full
        }
    }

    pub fn min_label_area(&self) -> f32 {
        match self {
            Self::Full => 2500.0,
            Self::Degraded => 5000.0,
            Self::Skip => 10000.0,
        }
    }
}

// Spring physics constants (critically damped)
const SPRING_STIFFNESS: f32 = 120.0;
const SPRING_DAMPING: f32 = 12.0;
const SETTLE_THRESHOLD: f32 = 0.5;
const REVEAL_FADE_DURATION: f32 = 0.1; // 100ms fade in

pub struct LayoutAnimator {
    states: Vec<RectAnimState>,
    elapsed: f32,
    pub is_animating: bool,
    pub tier: AnimationTier,
}

impl Default for LayoutAnimator {
    fn default() -> Self {
        Self {
            states: Vec::new(),
            elapsed: 0.0,
            is_animating: false,
            tier: AnimationTier::Full,
        }
    }
}

impl LayoutAnimator {
    /// Start animation for a new set of layout targets.
    /// `targets` are (x, y, w, h, index) tuples for each rect.
    /// `container_center` is the center point rects animate from.
    pub fn start(
        &mut self,
        targets: &[(f32, f32, f32, f32, usize)],
        container_center: (f32, f32),
        item_count: usize,
    ) {
        self.tier = AnimationTier::from_item_count(item_count);

        if self.tier == AnimationTier::Skip || targets.is_empty() {
            // Snap immediately - no animation
            self.states = targets
                .iter()
                .map(|&(x, y, w, h, index)| {
                    let rect = AnimRect { x, y, w, h, opacity: 1.0 };
                    RectAnimState {
                        current: rect,
                        target: rect,
                        velocity_x: 0.0,
                        velocity_y: 0.0,
                        velocity_w: 0.0,
                        velocity_h: 0.0,
                        reveal_delay: 0.0,
                        is_revealed: true,
                        is_settled: true,
                        index,
                    }
                })
                .collect();
            self.is_animating = false;
            self.elapsed = 0.0;
            return;
        }

        let num_batches = match self.tier {
            AnimationTier::Full => 6,
            AnimationTier::Degraded => 2,
            AnimationTier::Skip => 1,
        };
        let total_reveal_time = match self.tier {
            AnimationTier::Full => 0.15,   // 150ms spread
            AnimationTier::Degraded => 0.08,
            AnimationTier::Skip => 0.0,
        };

        // Items are already sorted largest-first by the treemap layout.
        // Assign reveal delays in batches.
        let batch_size = (targets.len() + num_batches - 1) / num_batches;

        self.states = targets
            .iter()
            .enumerate()
            .map(|(i, &(x, y, w, h, index))| {
                let batch = i / batch_size;
                let delay = batch as f32 * (total_reveal_time / num_batches as f32);

                let target = AnimRect { x, y, w, h, opacity: 1.0 };
                let current = AnimRect {
                    x: container_center.0,
                    y: container_center.1,
                    w: 0.0,
                    h: 0.0,
                    opacity: 0.0,
                };

                RectAnimState {
                    current,
                    target,
                    velocity_x: 0.0,
                    velocity_y: 0.0,
                    velocity_w: 0.0,
                    velocity_h: 0.0,
                    reveal_delay: delay,
                    is_revealed: false,
                    is_settled: false,
                    index,
                }
            })
            .collect();

        self.elapsed = 0.0;
        self.is_animating = true;
    }

    /// Update all animated rects. Returns true if still animating.
    pub fn update(&mut self, dt: f32) -> bool {
        if !self.is_animating {
            return false;
        }

        self.elapsed += dt;
        let elapsed = self.elapsed;
        let mut all_settled = true;

        for state in &mut self.states {
            // Check if this rect should start revealing
            if !state.is_revealed {
                if elapsed >= state.reveal_delay {
                    state.is_revealed = true;
                } else {
                    all_settled = false;
                    continue;
                }
            }

            if state.is_settled {
                continue;
            }

            // Opacity fade-in
            let reveal_elapsed = elapsed - state.reveal_delay;
            state.current.opacity = (reveal_elapsed / REVEAL_FADE_DURATION).min(1.0);

            // Spring physics for x
            let dx = state.current.x - state.target.x;
            let force_x = -SPRING_STIFFNESS * dx - SPRING_DAMPING * state.velocity_x;
            state.velocity_x += force_x * dt;
            state.current.x += state.velocity_x * dt;

            // Spring physics for y
            let dy = state.current.y - state.target.y;
            let force_y = -SPRING_STIFFNESS * dy - SPRING_DAMPING * state.velocity_y;
            state.velocity_y += force_y * dt;
            state.current.y += state.velocity_y * dt;

            // Spring physics for width
            let dw = state.current.w - state.target.w;
            let force_w = -SPRING_STIFFNESS * dw - SPRING_DAMPING * state.velocity_w;
            state.velocity_w += force_w * dt;
            state.current.w += state.velocity_w * dt;

            // Spring physics for height
            let dh = state.current.h - state.target.h;
            let force_h = -SPRING_STIFFNESS * dh - SPRING_DAMPING * state.velocity_h;
            state.velocity_h += force_h * dt;
            state.current.h += state.velocity_h * dt;

            // Check if settled
            let pos_delta = dx * dx + dy * dy + dw * dw + dh * dh;
            let vel_delta = state.velocity_x * state.velocity_x
                + state.velocity_y * state.velocity_y
                + state.velocity_w * state.velocity_w
                + state.velocity_h * state.velocity_h;

            if pos_delta < SETTLE_THRESHOLD && vel_delta < SETTLE_THRESHOLD
                && state.current.opacity >= 1.0
            {
                // Snap to target
                state.current = state.target;
                state.velocity_x = 0.0;
                state.velocity_y = 0.0;
                state.velocity_w = 0.0;
                state.velocity_h = 0.0;
                state.is_settled = true;
            } else {
                all_settled = false;
            }
        }

        if all_settled {
            self.is_animating = false;
        }

        self.is_animating
    }

    /// Get current animated rect states for rendering
    pub fn get_animated_rects(&self) -> &[RectAnimState] {
        &self.states
    }

    /// Snap all rects to their targets immediately (for user interaction during animation)
    pub fn finish_immediately(&mut self) {
        for state in &mut self.states {
            state.current = state.target;
            state.velocity_x = 0.0;
            state.velocity_y = 0.0;
            state.velocity_w = 0.0;
            state.velocity_h = 0.0;
            state.is_revealed = true;
            state.is_settled = true;
        }
        self.is_animating = false;
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_animation_skip_large_count() {
        let mut animator = LayoutAnimator::default();
        let targets = vec![(10.0, 20.0, 100.0, 50.0, 0)];
        animator.start(&targets, (400.0, 300.0), 1500);

        assert!(!animator.is_animating);
        assert_eq!(animator.tier, AnimationTier::Skip);
        let rects = animator.get_animated_rects();
        assert_eq!(rects.len(), 1);
        assert!((rects[0].current.x - 10.0).abs() < 0.01);
        assert!((rects[0].current.opacity - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_animation_full_starts_animating() {
        let mut animator = LayoutAnimator::default();
        let targets = vec![
            (10.0, 20.0, 100.0, 50.0, 0),
            (120.0, 20.0, 80.0, 50.0, 1),
        ];
        animator.start(&targets, (400.0, 300.0), 2);

        assert!(animator.is_animating);
        assert_eq!(animator.tier, AnimationTier::Full);

        // Current should start at container center
        let rects = animator.get_animated_rects();
        assert!((rects[0].current.x - 400.0).abs() < 0.01);
        assert!((rects[0].current.y - 300.0).abs() < 0.01);
    }

    #[test]
    fn test_animation_settles() {
        let mut animator = LayoutAnimator::default();
        let targets = vec![(100.0, 100.0, 200.0, 150.0, 0)];
        animator.start(&targets, (400.0, 300.0), 1);

        // Simulate many frames
        let dt = 1.0 / 60.0;
        for _ in 0..300 {
            if !animator.update(dt) {
                break;
            }
        }

        assert!(!animator.is_animating);
        let rects = animator.get_animated_rects();
        assert!((rects[0].current.x - 100.0).abs() < 1.0);
        assert!((rects[0].current.y - 100.0).abs() < 1.0);
    }

    #[test]
    fn test_finish_immediately() {
        let mut animator = LayoutAnimator::default();
        let targets = vec![
            (10.0, 20.0, 100.0, 50.0, 0),
            (120.0, 20.0, 80.0, 50.0, 1),
        ];
        animator.start(&targets, (400.0, 300.0), 2);
        assert!(animator.is_animating);

        animator.finish_immediately();
        assert!(!animator.is_animating);

        let rects = animator.get_animated_rects();
        assert!((rects[0].current.x - 10.0).abs() < 0.01);
        assert!((rects[1].current.x - 120.0).abs() < 0.01);
    }

    #[test]
    fn test_animation_tier_thresholds() {
        assert_eq!(AnimationTier::from_item_count(50), AnimationTier::Full);
        assert_eq!(AnimationTier::from_item_count(299), AnimationTier::Full);
        assert_eq!(AnimationTier::from_item_count(300), AnimationTier::Full);
        assert_eq!(AnimationTier::from_item_count(301), AnimationTier::Degraded);
        assert_eq!(AnimationTier::from_item_count(1000), AnimationTier::Degraded);
        assert_eq!(AnimationTier::from_item_count(1001), AnimationTier::Skip);
    }
}
