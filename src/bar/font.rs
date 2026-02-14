use std::ffi::CString;
use x11::xft::{XftColor, XftDraw, XftDrawStringUtf8, XftFont, XftFontOpenName};
use x11::xlib::_XDisplay;
use x11::xlib::{Colormap, Display, Drawable, Visual};
use x11::xrender::XRenderColor;

use crate::errors::X11Error;

enum DisplayAction {
    Flush,
    Sync,
}

enum FontAttribute {
    Height,
    Ascent,
}

pub struct Font {
    xft_font: *mut XftFont,
    display: *mut Display,
}

impl Font {
    pub fn new(display: *mut Display, screen: i32, font_name: &str) -> Result<Self, X11Error> {
        let font_name_cstr =
            CString::new(font_name).map_err(|_| X11Error::FontLoadFailed(font_name.to_string()))?;

        let xft_font = get_font(display, screen, font_name_cstr);

        if xft_font.is_null() {
            return Err(X11Error::FontLoadFailed(font_name.to_string()));
        }

        Ok(Font { xft_font, display })
    }

    pub fn height(&self) -> u16 {
        get_font_attribute(FontAttribute::Height, self.xft_font) as u16
    }

    pub fn ascent(&self) -> i16 {
        get_font_attribute(FontAttribute::Ascent, self.xft_font) as i16
    }

    pub fn text_width(&self, text: &str) -> u16 {
        get_text_width(self, text)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            if !self.xft_font.is_null() {
                x11::xft::XftFontClose(self.display, self.xft_font);
            }
        }
    }
}

pub struct FontDraw {
    xft_draw: *mut XftDraw,
}

impl FontDraw {
    pub fn new(
        display: *mut Display,
        drawable: Drawable,
        visual: *mut Visual,
        colormap: Colormap,
    ) -> Result<Self, X11Error> {
        let xft_draw = get_draw(display, drawable, visual, colormap);

        if xft_draw.is_null() {
            return Err(X11Error::DrawCreateFailed);
        }

        Ok(FontDraw { xft_draw })
    }

    pub fn draw_text(&self, font: &Font, color: u32, x: i16, y: i16, text: &str) {
        let red = ((color >> 16) & 0xFF) as u16;
        let green = ((color >> 8) & 0xFF) as u16;
        let blue = (color & 0xFF) as u16;

        let render_color = XRenderColor {
            red: red << 8 | red,
            green: green << 8 | green,
            blue: blue << 8 | blue,
            alpha: 0xFFFF,
        };

        do_draw(self.xft_draw, font, render_color, x, y, text);
    }

    pub fn flush(&self) {
        display_action(self.xft_draw, DisplayAction::Flush);
    }

    pub fn sync(&self) {
        display_action(self.xft_draw, DisplayAction::Sync);
    }
}

impl Drop for FontDraw {
    fn drop(&mut self) {
        unsafe {
            if !self.xft_draw.is_null() {
                x11::xft::XftDrawDestroy(self.xft_draw);
            }
        }
    }
}

pub struct DrawingSurface {
    font_draw: FontDraw,
    pixmap: x11::xlib::Pixmap,
    display: *mut Display,
}

impl DrawingSurface {
    pub fn new(
        display: *mut Display,
        window: x11::xlib::Drawable,
        width: u32,
        height: u32,
        visual: *mut Visual,
        colormap: Colormap,
    ) -> Result<Self, crate::errors::X11Error> {
        let depth = get_depth(display);
        let pixmap = get_pixmap(display, window, width, height, depth as u32);
        let font_draw = FontDraw::new(display, pixmap, visual, colormap)?;

        Ok(Self {
            font_draw,
            pixmap,
            display,
        })
    }

    pub fn pixmap(&self) -> x11::xlib::Pixmap {
        self.pixmap
    }

    pub fn font_draw(&self) -> &FontDraw {
        &self.font_draw
    }
}

impl Drop for DrawingSurface {
    fn drop(&mut self) {
        unsafe {
            x11::xft::XftDrawDestroy(self.font_draw.xft_draw);
            self.font_draw.xft_draw = std::ptr::null_mut();
            x11::xlib::XFreePixmap(self.display, self.pixmap);
        }
    }
}

fn get_font(display: *mut _XDisplay, screen: i32, font_name: CString) -> *mut XftFont {
    unsafe { XftFontOpenName(display, screen, font_name.as_ptr()) }
}

fn get_draw(
    display: *mut _XDisplay,
    drawable: Drawable,
    visual: *mut Visual,
    colormap: Colormap,
) -> *mut XftDraw {
    unsafe { x11::xft::XftDrawCreate(display, drawable, visual, colormap) }
}

fn get_font_attribute(attrib: FontAttribute, font_ref: *mut XftFont) -> i32 {
    unsafe {
        let font = &*font_ref;
        match attrib {
            FontAttribute::Height => font.height,

            FontAttribute::Ascent => font.ascent,
        }
    }
}

fn get_depth(display: *mut _XDisplay) -> i32 {
    unsafe { x11::xlib::XDefaultDepth(display, 0) }
}

fn get_pixmap(display: *mut _XDisplay, window: u64, width: u32, height: u32, depth: u32) -> u64 {
    unsafe { x11::xlib::XCreatePixmap(display, window, width, height, depth) }
}

fn get_text_width(font: &Font, text: &str) -> u16 {
    unsafe {
        let mut extents = std::mem::zeroed();
        x11::xft::XftTextExtentsUtf8(
            font.display,
            font.xft_font,
            text.as_ptr(),
            text.len() as i32,
            &mut extents,
        );
        extents.width
    }
}

fn display_action(font_draw: *mut XftDraw, action: DisplayAction) {
    unsafe {
        let display = x11::xft::XftDrawDisplay(font_draw);
        match action {
            DisplayAction::Flush => x11::xlib::XFlush(display),
            DisplayAction::Sync => x11::xlib::XSync(display, 1),
        };
    }
}

fn do_draw(font_draw: *mut XftDraw, font: &Font, color: XRenderColor, x: i16, y: i16, text: &str) {
    unsafe {
        let mut xft_color: XftColor = std::mem::zeroed();
        x11::xft::XftColorAllocValue(
            x11::xft::XftDrawDisplay(font_draw),
            x11::xft::XftDrawVisual(font_draw),
            x11::xft::XftDrawColormap(font_draw),
            &color,
            &mut xft_color,
        );

        XftDrawStringUtf8(
            font_draw,
            &xft_color,
            font.xft_font,
            x as i32,
            y as i32,
            text.as_ptr(),
            text.len() as i32,
        );

        x11::xft::XftColorFree(
            x11::xft::XftDrawDisplay(font_draw),
            x11::xft::XftDrawVisual(font_draw),
            x11::xft::XftDrawColormap(font_draw),
            &mut xft_color,
        );
    }
}
