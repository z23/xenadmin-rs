//! A picture the model holds: the renderer of an image block.

use xenadmin_ui::surface::Image;

use crate::kit::*;
use crate::render::Pass;

/// Draws `image` scaled to fit the width the pass has and the image's
/// maximum size, never enlarged and never distorted. The element has the
/// image role and the alt text as its name. Bytes that are no picture give
/// a framed line that says so, with the same key.
pub fn picture(pass: &Pass<'_>, image: &Image, window: &mut Window, cx: &mut App) -> AnyElement {
    let frame = pass.ui.cache.borrow_mut().image(pass.scope, image, window);
    let id = SharedString::from(image.key.clone());
    let alt = SharedString::from(image.alt.clone());
    let Some((frame, (width, height))) = frame else {
        let theme = cx.theme();
        return div()
            .id(id)
            .test_support()
            .role(Role::Image)
            .aria_label(alt.clone())
            .aria_description("The picture cannot be shown.")
            .w_full()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(SharedString::from(format!(
                "{alt}: the picture cannot be shown."
            )))
            .into_any_element();
    };
    let (width, height) = (width as f32, height as f32);
    let mut scale = (f32::from(pass.width) / width).min(1.0);
    if let Some((max_width, max_height)) = image.max_size {
        scale = scale
            .min(max_width as f32 / width)
            .min(max_height as f32 / height);
    }
    // The box has the picture's proportions, so the fit never letterboxes.
    // A flex parent: in block layout an image overflows its box.
    div()
        .id(id)
        .test_support()
        .role(Role::Image)
        .aria_label(alt)
        .flex()
        .flex_none()
        .w(px((width * scale).round().max(1.)))
        .h(px((height * scale).round().max(1.)))
        .child(img(frame).object_fit(ObjectFit::Contain).size_full())
        .into_any_element()
}
