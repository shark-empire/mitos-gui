//! Screen capture (Screenshots)
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElement; // Correct trait for Smithay 0.7
use smithay::utils::{Buffer, Physical, Rectangle, Size, Transform};
use image::RgbaImage;

pub fn take_screenshot<'a, E>(
    renderer: &mut GlesRenderer,
    output_size: Size<i32, Physical>,
    elements: &'a [E],
) -> Result<(), Box<dyn std::error::Error>>
where
    E: RenderElement<GlesRenderer> + 'a,
{
    // 1. Create offscreen buffer
    let buffer_size = Size::<i32, Buffer>::from((output_size.w, output_size.h));
    let mut texture = renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
    let mut target = renderer.bind(&mut texture)?;
    let mut tracker = OutputDamageTracker::new(output_size, 1.0, Transform::Normal);
    
    // 2. Render elements to the offscreen target (force full damage by passing age=0)
    // render_output is a method on OutputDamageTracker, not a free function.
    tracker.render_output(
        renderer,
        &mut target,
        0, // age
        elements,
        [0.0, 0.0, 0.0, 1.0], // Clear color
    )?;

    // 3. Copy framebuffer to CPU memory
    let region = Rectangle::new((0, 0).into(), buffer_size);
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
