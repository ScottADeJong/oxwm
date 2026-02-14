use crate::ColorScheme;
use crate::bar::font::{DrawingSurface, Font};
use crate::errors::X11Error;
use crate::layout::tabbed::TAB_BAR_HEIGHT;
use x11::xlib::_XDisplay;
use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

struct DrawElement {
    display: *mut _XDisplay,
    pixmap: x11::xlib::Pixmap,
    window: Option<x11::xlib::Drawable>,
    color: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub struct TabBar {
    window: Window,
    width: u16,
    height: u16,
    x_offset: i16,
    y_offset: i16,
    graphics_context: Gcontext,
    display: *mut x11::xlib::Display,
    surface: DrawingSurface,
    scheme_normal: ColorScheme,
    scheme_selected: ColorScheme,
}

impl TabBar {
    pub fn new(
        connection: &RustConnection,
        screen: &Screen,
        screen_num: usize,
        display: *mut x11::xlib::Display,
        _font: &Font,
        x: i16,
        y: i16,
        width: u16,
        scheme_normal: ColorScheme,
        scheme_selected: ColorScheme,
        cursor: u32,
    ) -> Result<Self, X11Error> {
        let window = connection.generate_id()?;
        let graphics_context = connection.generate_id()?;

        let height = TAB_BAR_HEIGHT as u16;

        connection.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            x,
            y,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(scheme_normal.background)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS)
                .override_redirect(1),
        )?;

        connection.create_gc(
            graphics_context,
            window,
            &CreateGCAux::new()
                .foreground(scheme_normal.foreground)
                .background(scheme_normal.background),
        )?;

        define_cursor(display, window as u64, cursor as u64);

        connection.map_window(window)?;
        connection.flush()?;

        let (visual, colormap) = get_visual_and_colormap(display, screen_num as i32);

        let surface = DrawingSurface::new(
            display,
            window as x11::xlib::Drawable,
            width as u32,
            height as u32,
            visual,
            colormap,
        )?;

        Ok(Self {
            window,
            width,
            height,
            x_offset: x,
            y_offset: y,
            graphics_context,
            display,
            surface,
            scheme_normal,
            scheme_selected,
        })
    }

    pub fn window(&self) -> Window {
        self.window
    }

    pub fn draw(
        &mut self,
        connection: &RustConnection,
        font: &Font,
        windows: &[(Window, String)],
        focused_window: Option<Window>,
    ) -> Result<(), X11Error> {
        connection.change_gc(
            self.graphics_context,
            &ChangeGCAux::new().foreground(self.scheme_normal.background),
        )?;
        connection.flush()?;

        draw_elements(DrawElement {
            display: self.display,
            pixmap: self.surface.pixmap(),
            window: None,
            color: self.scheme_normal.background,
            x: 0,
            y: 0,
            width: self.width as u32,
            height: self.height as u32,
        });

        if windows.is_empty() {
            self.copy_pixmap_to_window();
            return Ok(());
        }

        let tab_width = self.width / windows.len() as u16;
        let mut x_position: i16 = 0;

        for (index, &(window, ref title)) in windows.iter().enumerate() {
            let is_focused = Some(window) == focused_window;
            let scheme = if is_focused {
                &self.scheme_selected
            } else {
                &self.scheme_normal
            };

            let display_title = if title.is_empty() {
                format!("Window {}", index + 1)
            } else {
                title.clone()
            };

            let text_width = font.text_width(&display_title);
            let text_x = x_position + ((tab_width.saturating_sub(text_width)) / 2) as i16;

            let top_padding = 6;
            let text_y = top_padding + font.ascent();

            self.surface.font_draw().draw_text(
                font,
                scheme.foreground,
                text_x,
                text_y,
                &display_title,
            );

            if is_focused {
                let underline_height = 3;
                let underline_y = self.height as i16 - underline_height;

                draw_elements(DrawElement {
                    display: self.display,
                    pixmap: self.surface.pixmap(),
                    window: None,
                    color: scheme.underline,
                    x: x_position as i32,
                    y: underline_y as i32,
                    width: tab_width as u32,
                    height: underline_height as u32,
                });
            }

            x_position += tab_width as i16;
        }

        self.copy_pixmap_to_window();
        Ok(())
    }

    fn copy_pixmap_to_window(&self) {
        draw_elements(DrawElement {
            display: self.display,
            pixmap: self.surface.pixmap(),
            window: Some(self.window as u64),
            color: 0,
            x: 0,
            y: 0,
            width: self.width as u32,
            height: self.height as u32,
        });
    }

    pub fn get_clicked_window(&self, windows: &[(Window, String)], click_x: i16) -> Option<Window> {
        if windows.is_empty() {
            return None;
        }

        let tab_width = self.width / windows.len() as u16;
        let tab_index = (click_x as u16 / tab_width) as usize;

        windows.get(tab_index).map(|&(win, _)| win)
    }

    pub fn reposition(
        &mut self,
        connection: &RustConnection,
        x: i16,
        y: i16,
        width: u16,
    ) -> Result<(), X11Error> {
        self.x_offset = x;
        self.y_offset = y;
        self.width = width;

        connection.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .width(width as u32),
        )?;

        let (visual, colormap) = get_visual_and_colormap(self.display, 0);

        self.surface = DrawingSurface::new(
            self.display,
            self.window as x11::xlib::Drawable,
            width as u32,
            self.height as u32,
            visual,
            colormap,
        )?;

        connection.flush()?;
        Ok(())
    }

    pub fn hide(&self, connection: &RustConnection) -> Result<(), X11Error> {
        connection.unmap_window(self.window)?;
        connection.flush()?;
        Ok(())
    }

    pub fn show(&self, connection: &RustConnection) -> Result<(), X11Error> {
        connection.map_window(self.window)?;
        connection.flush()?;
        Ok(())
    }
}

fn draw_elements(element: DrawElement) {
    unsafe {
        let gc = x11::xlib::XCreateGC(element.display, element.pixmap, 0, std::ptr::null_mut());
        match element.window {
            Some(w) => {
                x11::xlib::XCopyArea(
                    element.display,
                    element.pixmap,
                    w,
                    gc,
                    element.x,
                    element.y,
                    element.width,
                    element.height,
                    0,
                    0,
                );
                x11::xlib::XFreeGC(element.display, gc);
                x11::xlib::XSync(element.display, 1);
            }
            None => {
                x11::xlib::XSetForeground(element.display, gc, element.color as u64);
                x11::xlib::XFillRectangle(
                    element.display,
                    element.pixmap,
                    gc,
                    element.x,
                    element.y,
                    element.width,
                    element.height,
                );
                x11::xlib::XFreeGC(element.display, gc);
            }
        }
    }
}

fn define_cursor(display: *mut _XDisplay, window: u64, cursor: u64) {
    unsafe {
        x11::xlib::XDefineCursor(display, window, cursor);
    }
}

fn get_visual_and_colormap(
    display: *mut _XDisplay,
    screen_num: i32,
) -> (*mut x11::xlib::Visual, u64) {
    unsafe {
        (
            x11::xlib::XDefaultVisual(display, screen_num),
            x11::xlib::XDefaultColormap(display, screen_num),
        )
    }
}
