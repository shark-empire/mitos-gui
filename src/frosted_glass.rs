//! True Frosted Glass implementation using custom GLES shaders.

use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformValue,
};
use smithay::backend::renderer::element::{Element, RenderElement, Id};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Buffer, Physical, Rectangle, Transform};

/// The GLSL fragment shader for the frosted glass effect.
/// It performs a 9-tap Gaussian cross-blur and applies tint + highlights.
const FROSTED_GLASS_SHADER: &str = r#"
//_DEFINES
precision mediump float;
varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;
uniform float tint; // Smithay passes this (0.0 or 1.0), we use our own color
uniform vec2 u_tex_size;
uniform vec4 u_tint_color;
uniform vec4 u_border_color;

void main() {
    vec2 texel = 1.0 / u_tex_size;
    vec4 sum = vec4(0.0);
    
    // 9-tap Gaussian weights
    float w0 = 0.227027;
    float w1 = 0.1945946;
    float w2 = 0.1216216;
    float w3 = 0.054054;
    float w4 = 0.016216;
    
    // Center
    sum += texture2D(tex, v_coords) * w0;
    
    // 1-pixel radius
    sum += texture2D(tex, v_coords + vec2( 1.0,  0.0) * texel) * w1;
    sum += texture2D(tex, v_coords + vec2(-1.0,  0.0) * texel) * w1;
    sum += texture2D(tex, v_coords + vec2( 0.0,  1.0) * texel) * w1;
    sum += texture2D(tex, v_coords + vec2( 0.0, -1.0) * texel) * w1;
    
    // 2-pixel radius
    sum += texture2D(tex, v_coords + vec2( 2.0,  0.0) * texel) * w2;
    sum += texture2D(tex, v_coords + vec2(-2.0,  0.0) * texel) * w2;
    sum += texture2D(tex, v_coords + vec2( 0.0,  2.0) * texel) * w2;
    sum += texture2D(tex, v_coords + vec2( 0.0, -2.0) * texel) * w2;
    
    // 3-pixel radius
    sum += texture2D(tex, v_coords + vec2( 3.0,  0.0) * texel) * w3;
    sum += texture2D(tex, v_coords + vec2(-3.0,  0.0) * texel) * w3;
    sum += texture2D(tex, v_coords + vec2( 0.0,  3.0) * texel) * w3;
    sum += texture2D(tex, v_coords + vec2( 0.0, -3.0) * texel) * w3;
    
    // 4-pixel radius
    sum += texture2D(tex, v_coords + vec2( 4.0,  0.0) * texel) * w4;
    sum += texture2D(tex, v_coords + vec2(-4.0,  0.0) * texel) * w4;
    sum += texture2D(tex, v_coords + vec2( 0.0,  4.0) * texel) * w4;
    sum += texture2D(tex, v_coords + vec2( 0.0, -4.0) * texel) * w4;

    // Normalize cross-blur (divide by 2 to prevent double-brightening)
    vec4 blurred = sum * 0.5; 
    
    // Apply theme tint color
    vec4 colored = blurred * u_tint_color;
    
    // Specular highlight on the top edge (glass reflection)
    float highlight = smoothstep(0.0, 0.05, v_coords.y) * (1.0 - smoothstep(0.05, 0.1, v_coords.y));
    colored += vec4(1.0, 1.0, 1.0, highlight * 0.3);

    // Subtle bottom border
    float border = step(0.99, v_coords.y) * u_border_color.a;
    colored = mix(colored, u_border_color, border * 0.5);

    gl_FragColor = colored * alpha;
}
"#;

/// Compiles the frosted glass shader program.
pub fn compile_frosted_program(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        FROSTED_GLASS_SHADER,
        &[
            smithay::backend::renderer::gles::UniformName::new("u_tex_size", smithay::backend::renderer::gles::UniformType::_2f),
            smithay::backend::renderer::gles::UniformName::new("u_tint_color", smithay::backend::renderer::gles::UniformType::_4f),
            smithay::backend::renderer::gles::UniformName::new("u_border_color", smithay::backend::renderer::gles::UniformType::_4f),
        ],
    )
}

/// The render element that draws the frosted glass panel.
pub struct FrostedGlassElement {
    pub geometry: Rectangle<i32, Physical>,
    pub bg_texture: GlesTexture,
    pub program: GlesTexProgram,
    pub tint: [f32; 4],
    pub border: [f32; 4],
    pub id: Id,
}

impl FrostedGlassElement {
    pub fn new(
        geometry: Rectangle<i32, Physical>,
        bg_texture: GlesTexture,
        program: GlesTexProgram,
        tint: [f32; 4],
        border: [f32; 4],
    ) -> Self {
        Self {
            geometry,
            bg_texture,
            program,
            tint,
            border,
            id: Id::new(), // Generate a persistent unique ID once per element
        }
    }
}

// 1. Basic Geometry and Commit Tracking
impl Element for FrostedGlassElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        // Map the physical geometry to the texture coordinates
        Rectangle::new(
            (self.geometry.loc.x as f64, self.geometry.loc.y as f64).into(),
            self.geometry.size.to_f64(),
        )
    }

    fn geometry(&self, _scale: smithay::utils::Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }
}

// 2. The GlesRenderer Drawing Logic
impl RenderElement<GlesRenderer> for FrostedGlassElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let tex_size = self.bg_texture.size();
        
        let uniforms = vec![
            Uniform::new("u_tex_size", UniformValue::_2f(tex_size.w as f32, tex_size.h as f32)),
            Uniform::new("u_tint_color", UniformValue::_4f(self.tint[0], self.tint[1], self.tint[2], self.tint[3])),
            Uniform::new("u_border_color", UniformValue::_4f(self.border[0], self.border[1], self.border[2], self.border[3])),
        ];

        frame.render_texture_from_to(
            &self.bg_texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            1.0, // alpha
            Some(&self.program),
            &uniforms,
        )
    }
}
