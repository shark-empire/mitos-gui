//! Screen capture (Screenshots)
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen, Renderer};
use smithay::backend::renderer::damage::{OutputDamageTracker, render_output};
use smithay::utils::{Physical, Rectangle, Size, Transform};
use image::RgbaImage;

pub fn take_screenshot<E>(
    renderer: &mut GlesRenderer,
    output_size: Size<i32, Physical>,
    elements: &[E],
) -> Result<(), Box<dyn std::error::Error>>
where
    E: smithay::backend::renderer::element::Element<GlesRenderer>,
{
    // 1. Create offscreen buffer
    let mut texture = renderer.create_buffer(Fourcc::Abgr8888, output_size)?;
    let mut target = renderer.bind(&mut texture)?;
    let mut tracker = OutputDamageTracker::new(output_size, 1.0, Transform::Normal);
    
    // 2. Render elements to the offscreen target (force full damage)
    let _ = render_output(
        renderer,
        &mut target,
        &mut tracker,
        0,
        [0.0, 0.0, 0.0, 1.0], // Clear color
        elements.iter(),
    )?;

    // 3. Copy framebuffer to CPU memory
    let region = Rectangle::new((0, 0).into(), output_size);
    let mapping = renderer.copy_framebuffer(&target, region, Fourcc::Abgr8888)?;
    let data = renderer.map_texture(&mapping)?;
    
    // 4. Save to PNG
    let img = RgbaImage::from_raw(output_size.w as u32, output_size.h as u32, data.to_vec())
        .ok_or("Failed to create image buffer")?;
        
    let path = dirs::picture_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(format!("mitos_screenshot_{}.png", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")));
        
    img.save(&path)?;
    println!("MITOS GUI: Screenshot saved to {:?}", path);
    Ok(())
}
