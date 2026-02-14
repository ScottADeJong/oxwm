use super::blocks::Block;
use super::font::{DrawingSurface, Font};
use crate::Config;
use crate::errors::X11Error;
use crate::monitor::ScreenInfo;
use std::time::Instant;
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

struct BarObject<'a> {
    font: &'a Font,
    color: u32,
    x: i16,
    y: i16,
    text: String,
}

pub struct Bar {
    window: Window,
    width: u16,
    height: u16,
    graphics_context: Gcontext,
    surface: DrawingSurface,

    tag_widths: Vec<u16>,
    needs_redraw: bool,

    blocks: Vec<Box<dyn Block>>,
    block_last_updates: Vec<Instant>,
    block_underlines: Vec<bool>,
    status_text: String,

    tags: Vec<String>,
    scheme_normal: crate::ColorScheme,
    scheme_occupied: crate::ColorScheme,
    scheme_selected: crate::ColorScheme,
    scheme_urgent: crate::ColorScheme,
    hide_vacant_tags: bool,
    last_occupied_tags: u32,
    last_current_tags: u32,
}

impl Bar {
    pub fn new(
        connection: &RustConnection,
        screen: &Screen,
        screen_num: usize,
        config: &Config,
        display: *mut x11::xlib::Display,
        font: &Font,
        screen_info: &ScreenInfo,
        cursor: u32,
    ) -> Result<Self, X11Error> {
        let window = connection.generate_id()?;
        let graphics_context = connection.generate_id()?;

        let height = (font.height() as f32 * 1.4) as u16;

        connection.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            screen_info.x as i16,
            screen_info.y as i16,
            screen_info.width as u16,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(config.scheme_normal.background)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS)
                .override_redirect(1),
        )?;

        connection.create_gc(
            graphics_context,
            window,
            &CreateGCAux::new()
                .foreground(config.scheme_normal.foreground)
                .background(config.scheme_normal.background),
        )?;

        define_cursor(display, window as u64, cursor as u64);

        connection.map_window(window)?;
        connection.flush()?;

        let (visual, colormap) = get_visual_and_colormap(display, screen_num as i32);

        let surface = DrawingSurface::new(
            display,
            window as x11::xlib::Drawable,
            screen_info.width as u32,
            height as u32,
            visual,
            colormap,
        )?;

        let horizontal_padding = (font.height() as f32 * 0.4) as u16;

        let tag_widths = config
            .tags
            .iter()
            .map(|tag| {
                let text_width = font.text_width(tag);
                text_width + (horizontal_padding * 2)
            })
            .collect();

        let blocks: Vec<Box<dyn Block>> = config
            .status_blocks
            .iter()
            .map(|block_config| block_config.to_block())
            .collect();

        let block_underlines: Vec<bool> = config
            .status_blocks
            .iter()
            .map(|block_config| block_config.underline)
            .collect();

        let block_last_updates = vec![Instant::now(); blocks.len()];

        Ok(Bar {
            window,
            width: screen_info.width as u16,
            height,
            graphics_context,
            surface,
            tag_widths,
            needs_redraw: true,
            blocks,
            block_last_updates,
            block_underlines,
            status_text: String::new(),
            tags: config.tags.clone(),
            scheme_normal: config.scheme_normal,
            scheme_occupied: config.scheme_occupied,
            scheme_selected: config.scheme_selected,
            scheme_urgent: config.scheme_urgent,
            hide_vacant_tags: config.hide_vacant_tags,
            last_occupied_tags: 0,
            last_current_tags: 0,
        })
    }

    pub fn window(&self) -> Window {
        self.window
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn invalidate(&mut self) {
        self.needs_redraw = true;
    }

    pub fn update_blocks(&mut self) {
        let now = Instant::now();
        let mut changed = false;

        for (i, block) in self.blocks.iter_mut().enumerate() {
            let elapsed = now.duration_since(self.block_last_updates[i]);

            if elapsed >= block.interval() && block.content().is_ok() {
                self.block_last_updates[i] = now;
                changed = true;
            }
        }

        if changed {
            let mut parts = Vec::new();
            for block in &mut self.blocks {
                if let Ok(text) = block.content() {
                    parts.push(text);
                }
            }
            self.status_text = parts.join("");
            self.needs_redraw = true;
        }
    }

    pub fn update_tags(&mut self) {}

    pub fn draw(
        &mut self,
        connection: &RustConnection,
        font: &Font,
        display: *mut x11::xlib::Display,
        current_tags: u32,
        occupied_tags: u32,
        urgent_tags: u32,
        draw_blocks: bool,
        layout_symbol: &str,
        keychord_indicator: Option<&str>,
        focused_title: Option<String>,
    ) -> Result<(), X11Error> {
        if !self.needs_redraw {
            return Ok(());
        }

        connection.change_gc(
            self.graphics_context,
            &ChangeGCAux::new().foreground(self.scheme_normal.background),
        )?;
        connection.flush()?;

        draw_elements(DrawElement {
            display,
            pixmap: self.surface.pixmap(),
            window: None,
            color: self.scheme_normal.background,
            x: 0,
            y: 0,
            width: self.width as u32,
            height: self.height as u32,
        });

        self.last_occupied_tags = occupied_tags;
        self.last_current_tags = current_tags;

        let mut x_position: i16 = 0;
        let mut bar_objects: Vec<BarObject> = Vec::new();

        for (tag_index, tag) in self.tags.iter().enumerate() {
            let tag_mask = 1 << tag_index;
            let is_selected = (current_tags & tag_mask) != 0;
            let is_occupied = (occupied_tags & tag_mask) != 0;
            let is_urgent = (urgent_tags & tag_mask) != 0;

            if self.hide_vacant_tags && !is_occupied && !is_selected {
                continue;
            }

            let tag_width = self.tag_widths[tag_index];

            let scheme = if is_selected {
                &self.scheme_selected
            } else if is_urgent {
                &self.scheme_urgent
            } else if is_occupied {
                &self.scheme_occupied
            } else {
                &self.scheme_normal
            };

            let text_width = font.text_width(tag);
            let text_x = x_position + ((tag_width - text_width) / 2) as i16;

            let top_padding = 4;
            let text_y = top_padding + font.ascent();
            bar_objects.push(BarObject {
                font,
                color: scheme.foreground,
                x: text_x,
                y: text_y,
                text: tag.to_string(),
            });

            if is_selected || is_urgent {
                let font_height = font.height();
                let underline_height = font_height / 8;
                let bottom_gap = 3;
                let underline_y = self.height as i16 - underline_height as i16 - bottom_gap;

                let underline_padding = 4;
                let underline_width = tag_width - underline_padding;
                let underline_x = x_position + (underline_padding / 2) as i16;

                draw_elements(DrawElement {
                    display,
                    pixmap: self.surface.pixmap(),
                    window: None,
                    color: scheme.underline,
                    x: underline_x as i32,
                    y: underline_y as i32,
                    width: underline_width as u32,
                    height: underline_height as u32,
                });
            }

            x_position += tag_width as i16;
        }

        x_position += 10;

        let text_x = x_position;
        let top_padding = 4;
        let text_y = top_padding + font.ascent();

        bar_objects.push(BarObject {
            font,
            color: self.scheme_normal.foreground,
            x: text_x,
            y: text_y,
            text: layout_symbol.to_string(),
        });

        x_position += font.text_width(layout_symbol) as i16;

        if let Some(indicator) = keychord_indicator {
            x_position += 10;

            let text_x = x_position;
            let text_y = top_padding + font.ascent();

            bar_objects.push(BarObject {
                font,
                color: self.scheme_normal.foreground,
                x: text_x,
                y: text_y,
                text: indicator.to_string(),
            });
        }

        let mut end_of_blocks_x = self.width as i16;

        if draw_blocks && !self.status_text.is_empty() {
            let padding = 10;
            let mut x_position = self.width as i16 - padding;

            for (i, block) in self.blocks.iter_mut().enumerate().rev() {
                if let Ok(text) = block.content() {
                    let text_width = font.text_width(&text);
                    x_position -= text_width as i16;

                    let top_padding = 4;
                    let text_y = top_padding + font.ascent();

                    bar_objects.push(BarObject {
                        font,
                        color: block.color(),
                        x: x_position,
                        y: text_y,
                        text,
                    });

                    if self.block_underlines[i] {
                        let font_height = font.height();
                        let underline_height = font_height / 8;
                        let bottom_gap = 3;
                        let underline_y = self.height as i16 - underline_height as i16 - bottom_gap;

                        let underline_padding = 8;
                        let underline_width = text_width + underline_padding;
                        let underline_x = x_position - (underline_padding / 2) as i16;

                        draw_elements(DrawElement {
                            display,
                            pixmap: self.surface.pixmap(),
                            window: None,
                            color: block.color(),
                            x: underline_x as i32,
                            y: underline_y as i32,
                            width: underline_width as u32,
                            height: underline_height as u32,
                        });
                    }
                }
            }
            end_of_blocks_x = x_position;
        }

        if let Some(title) = focused_title {
            let end_of_layout_x = x_position + 10;
            let middle_remaining = (end_of_blocks_x - end_of_layout_x) / 2;
            let mut title_width = font.text_width(&title) as i16;
            let mut end_of_title = title.len();

            let title_start = match (middle_remaining - title_width / 2) < end_of_layout_x {
                true => end_of_layout_x + 10,
                false => middle_remaining - title_width / 2,
            };

            // possibly a better way to do this, but since not all fonts are monospace
            // I figured this was the safest and should rarely run more than one or two iterrations
            while title_start + title_width > end_of_blocks_x {
                end_of_title -= 1;
                title_width = font.text_width(&title[..end_of_title]) as i16;
            }

            bar_objects.push(BarObject {
                font,
                color: self.scheme_selected.foreground,
                x: title_start,
                y: text_y,
                text: title[..end_of_title].to_string(),
            });
        }

        for object in bar_objects {
            self.surface.font_draw().draw_text(
                object.font,
                object.color,
                object.x,
                object.y,
                &object.text,
            );
        }

        draw_elements(DrawElement {
            display,
            pixmap: self.surface.pixmap(),
            window: Some(self.window as x11::xlib::Drawable),
            color: 0,
            x: 0,
            y: 0,
            width: self.width as u32,
            height: self.height as u32,
        });

        self.needs_redraw = false;

        Ok(())
    }

    pub fn handle_click(&self, click_x: i16) -> Option<usize> {
        let mut current_x_position = 0;

        for (tag_index, &tag_width) in self.tag_widths.iter().enumerate() {
            let tag_mask = 1 << tag_index;
            let is_selected = (self.last_current_tags & tag_mask) != 0;
            let is_occupied = (self.last_occupied_tags & tag_mask) != 0;

            if self.hide_vacant_tags && !is_occupied && !is_selected {
                continue;
            }

            if click_x >= current_x_position && click_x < current_x_position + tag_width as i16 {
                return Some(tag_index);
            }
            current_x_position += tag_width as i16;
        }
        None
    }

    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn update_from_config(&mut self, config: &Config) {
        self.blocks = config
            .status_blocks
            .iter()
            .map(|block_config| block_config.to_block())
            .collect();

        self.block_underlines = config
            .status_blocks
            .iter()
            .map(|block_config| block_config.underline)
            .collect();

        self.block_last_updates = vec![Instant::now(); self.blocks.len()];

        self.tags = config.tags.clone();
        self.scheme_normal = config.scheme_normal;
        self.scheme_occupied = config.scheme_occupied;
        self.scheme_selected = config.scheme_selected;
        self.scheme_urgent = config.scheme_urgent;
        self.hide_vacant_tags = config.hide_vacant_tags;

        self.status_text.clear();
        self.needs_redraw = true;
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
