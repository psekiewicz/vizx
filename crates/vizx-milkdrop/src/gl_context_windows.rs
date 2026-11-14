//! An OpenGL context that draws off-screen, created with WGL (the Windows
//! OpenGL interface).
//!
//! projectM draws with OpenGL, while the VizX window is drawn with wgpu
//! (DirectX 12 on Windows). The two cannot share a surface, so projectM gets
//! its own OpenGL context attached to a window that is never shown. The
//! finished picture is read back from it as plain pixels.
//!
//! This is the only platform-specific code of the crate.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetDC, HDC, ReleaseDC};
use windows_sys::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, GL_BACK, GL_PACK_ALIGNMENT, GL_RENDERER, GL_RGBA, GL_UNSIGNED_BYTE,
    GL_VERSION, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA,
    PIXELFORMATDESCRIPTOR, SetPixelFormat, glFinish, glGetString, glPixelStorei, glReadBuffer,
    glReadPixels, glViewport, wglCreateContext, wglDeleteContext, wglMakeCurrent,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_OWNDC, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WNDCLASSW, WS_POPUP,
};

/// UTF-16 text with a terminating zero, as Windows functions expect.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // The window is never shown and needs no behavior of its own.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// A hidden window with an OpenGL context that is current on this thread.
///
/// OpenGL contexts belong to the thread that made them current, so this
/// type must be created, used and dropped on one and the same thread. The
/// raw pointers inside make the compiler enforce that: the type cannot be
/// sent to another thread.
pub struct OffscreenGlContext {
    window: HWND,
    device_context: HDC,
    gl_context: HGLRC,
    width: u32,
    height: u32,
}

impl OffscreenGlContext {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        // SAFETY: plain Win32 calls with valid arguments; every returned
        // handle is checked before it is used, and released again on error.
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let class_name = wide("VizxMilkdropOffscreen");
            let window_class = WNDCLASSW {
                // CS_OWNDC: the window keeps one device context for its
                // whole life, which OpenGL requires.
                style: CS_OWNDC,
                lpfnWndProc: Some(window_procedure),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            // Registering twice fails harmlessly; the class then exists.
            RegisterClassW(&window_class);

            // WS_POPUP has no border or title bar, so the drawable area is
            // exactly width x height. The window is created hidden.
            let window = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                width as i32,
                height as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            if window.is_null() {
                return Err("could not create the off-screen window".to_owned());
            }

            let device_context = GetDC(window);
            if device_context.is_null() {
                DestroyWindow(window);
                return Err("could not get a device context".to_owned());
            }

            let pixel_format = PIXELFORMATDESCRIPTOR {
                nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cDepthBits: 24,
                cStencilBits: 8,
                ..std::mem::zeroed()
            };
            let format_index = ChoosePixelFormat(device_context, &pixel_format);
            if format_index == 0 || SetPixelFormat(device_context, format_index, &pixel_format) == 0
            {
                ReleaseDC(window, device_context);
                DestroyWindow(window);
                return Err("no suitable OpenGL pixel format".to_owned());
            }

            let gl_context = wglCreateContext(device_context);
            if gl_context.is_null() || wglMakeCurrent(device_context, gl_context) == 0 {
                if !gl_context.is_null() {
                    wglDeleteContext(gl_context);
                }
                ReleaseDC(window, device_context);
                DestroyWindow(window);
                return Err("could not create an OpenGL context".to_owned());
            }

            glViewport(0, 0, width as i32, height as i32);
            Ok(Self {
                window,
                device_context,
                gl_context,
                width,
                height,
            })
        }
    }

    /// The OpenGL version and graphics card of this context, for
    /// diagnostics, e.g. `4.3.0 - Build 20.19.15.4835 (Intel(R) HD Graphics 4600)`.
    pub fn description(&self) -> String {
        // SAFETY: the context is current; `glGetString` returns either null
        // or a zero-terminated string owned by the driver.
        let read = |name| unsafe {
            let text = glGetString(name);
            if text.is_null() {
                "unknown".to_owned()
            } else {
                std::ffi::CStr::from_ptr(text.cast())
                    .to_string_lossy()
                    .into_owned()
            }
        };
        format!("{} ({})", read(GL_VERSION), read(GL_RENDERER))
    }

    /// Copies the picture that was just drawn into `pixels` as RGBA bytes,
    /// top row first. `pixels` is resized to `width * height * 4`.
    pub fn read_pixels(&self, pixels: &mut Vec<u8>) {
        let row_length = self.width as usize * 4;
        pixels.resize(row_length * self.height as usize, 0);

        // SAFETY: the context is current on this thread, and the buffer is
        // exactly as large as the area being read.
        unsafe {
            glFinish(); // wait until the graphics card has finished drawing
            glReadBuffer(GL_BACK);
            glPixelStorei(GL_PACK_ALIGNMENT, 1);
            glReadPixels(
                0,
                0,
                self.width as i32,
                self.height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }

        // OpenGL's first row is the bottom of the picture; flip it so the
        // first row is the top, as images are stored everywhere else.
        let height = self.height as usize;
        for row in 0..height / 2 {
            let (upper, lower) = pixels.split_at_mut((height - 1 - row) * row_length);
            upper[row * row_length..(row + 1) * row_length]
                .swap_with_slice(&mut lower[..row_length]);
        }
    }
}

impl Drop for OffscreenGlContext {
    fn drop(&mut self) {
        // SAFETY: the handles were created in `new` and are released once.
        unsafe {
            wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
            wglDeleteContext(self.gl_context);
            ReleaseDC(self.window, self.device_context);
            DestroyWindow(self.window);
        }
    }
}
