use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    FreeLibrary, GetLastError, ERROR_CLASS_ALREADY_EXISTS, HMODULE, HWND, LPARAM, LRESULT, POINT,
    RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetStockObject, InflateRect, InvalidateRect, MapWindowPoints,
    RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, UpdateWindow, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    FF_SWISS, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT, HPEN, NULL_PEN, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetModuleHandleW, GetProcAddress, LoadLibraryW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETCUEBANNER, ODS_SELECTED, ODT_BUTTON};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetDlgCtrlID,
    GetForegroundWindow, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, LoadCursorW, MoveWindow,
    PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, UnregisterClassW, BS_DEFPUSHBUTTON, BS_OWNERDRAW, CREATESTRUCTW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, DM_SETDEFID, GWLP_USERDATA, HWND_TOP, IDC_ARROW,
    MINMAXINFO, MSG, SM_CXSCREEN, SM_CYSCREEN, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CREATE,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_PAINT, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

const CLASS: &str = "DeviceOutFeedback";

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ID_TITLE: usize = 100;
const ID_DESC_LBL: usize = 101;
const ID_CONTACT_LBL: usize = 102;

const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_WANTRETURN: u32 = 0x1000;
const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;
const EM_SETMARGINS: u32 = 0x00D3;
const EC_LEFTMARGIN: u32 = 0x0001;
const EC_RIGHTMARGIN: u32 = 0x0002;

const CLR_BG: u32 = 0x000B_0909;
const CLR_FIELD: u32 = 0x0012_0F0F;
const CLR_TEXT: u32 = 0x00FA_FAFA;
const CLR_MUTED: u32 = 0x00AA_A1A1;
const CLR_PRIMARY: u32 = 0x00FA_FAFA;
const CLR_PRIMARY_DOWN: u32 = 0x00D8_D4D4;
const CLR_PRIMARY_TEXT: u32 = 0x000B_0909;
const CLR_SECONDARY_DOWN: u32 = 0x002A_2727;
const CLR_BORDER: u32 = 0x0046_3F3F;
const CLR_RING: u32 = 0x002A_2727;
const CLR_RING_FOCUS: u32 = 0x005B_5252;

const MARGIN: i32 = 20;
const TITLE_H: i32 = 22;
const LBL_H: i32 = 16;
const LBL_FIELD_GAP: i32 = 8;
const GROUP_GAP: i32 = 14;
const BTN_W: i32 = 92;
const BTN_H: i32 = 32;
const BTN_GAP: i32 = 8;
const CONTACT_FIELD_H: i32 = 36;
const FIELD_PAD: i32 = 5;
const TEXT_MARGIN: i32 = 8;
const RING_RADIUS: i32 = 6;

const FONT_TITLE_PX: i32 = 16;
const FONT_LBL_PX: i32 = 14;
const FONT_EDIT_PX: i32 = 15;
const FONT_BTN_PX: i32 = 14;

pub type PromptWindow = Arc<AtomicIsize>;

pub fn close_prompt(window: &PromptWindow) {
    let hwnd = window.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

struct Feedback {
    desc_label: Vec<u16>,
    contact_hint: Vec<u16>,
    dpi: u32,
    title_lbl: HWND,
    desc_lbl: HWND,
    contact_lbl: HWND,
    desc_edit: HWND,
    contact_edit: HWND,
    ok_btn: HWND,
    cancel_btn: HWND,
    font_title: HFONT,
    font_lbl: HFONT,
    font_edit: HFONT,
    font_btn: HFONT,
    bg_brush: HBRUSH,
    field_brush: HBRUSH,
    focus_edit: HWND,
    result: Option<(String, String)>,
}

pub fn run_feedback(kind_is_bug: bool, window: &PromptWindow) -> Option<(String, String)> {
    let t = deviceout_i18n::t();
    let (title, desc_label) = if kind_is_bug {
        (t.prompt_bug_title, t.prompt_describe_bug)
    } else {
        (t.prompt_feature_title, t.prompt_describe_feature)
    };
    unsafe { feedback_impl(title, desc_label, t.prompt_contact_hint, window) }
}

fn dp(v: i32, dpi: u32) -> i32 {
    ((v as i64 * dpi as i64 + 48) / 96) as i32
}

unsafe fn own_module() -> HMODULE {
    unsafe {
        let mut module: HMODULE = null_mut();
        let flags =
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        let anchor = wndproc as *const u16;
        if GetModuleHandleExW(flags, anchor, &mut module) == 0 {
            return GetModuleHandleW(null_mut());
        }
        module
    }
}

unsafe fn feedback_impl(
    title: &str,
    desc_label: &str,
    contact_hint: &str,
    window: &PromptWindow,
) -> Option<(String, String)> {
    unsafe {
        let _dpi = ThreadDpiScope::per_monitor_v2();

        let instance = own_module();
        let class_w = wide(CLASS);
        let bg_brush = CreateSolidBrush(CLR_BG);
        let field_brush = CreateSolidBrush(CLR_FIELD);

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null_mut(),
            lpszClassName: class_w.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            DeleteObject(bg_brush);
            DeleteObject(field_brush);
            return None;
        }

        let feedback = Box::into_raw(Box::new(Feedback {
            desc_label: wide(desc_label),
            contact_hint: wide(contact_hint),
            dpi: 96,
            title_lbl: null_mut(),
            desc_lbl: null_mut(),
            contact_lbl: null_mut(),
            desc_edit: null_mut(),
            contact_edit: null_mut(),
            ok_btn: null_mut(),
            cancel_btn: null_mut(),
            font_title: null_mut(),
            font_lbl: null_mut(),
            font_edit: null_mut(),
            font_btn: null_mut(),
            bg_brush,
            field_brush,
            focus_edit: null_mut(),
            result: None,
        }));

        let title_w = wide(title);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class_w.as_ptr(),
            title_w.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            feedback.cast(),
        );
        if hwnd.is_null() {
            let feedback = Box::from_raw(feedback);
            DeleteObject(feedback.bg_brush);
            DeleteObject(feedback.field_brush);
            UnregisterClassW(class_w.as_ptr(), instance);
            return None;
        }
        window.store(hwnd as isize, Ordering::SeqCst);

        place_window(hwnd);
        enable_dark_title_bar(hwnd);
        enable_rounded_corners(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        SetFocus((*feedback).desc_edit);

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_KEYDOWN {
                if msg.wParam as u16 == VK_ESCAPE {
                    DestroyWindow(hwnd);
                    continue;
                }
                if msg.wParam as u16 == VK_RETURN && GetKeyState(VK_CONTROL as i32) < 0 {
                    let focus = GetFocus();
                    if focus == (*feedback).desc_edit || focus == (*feedback).contact_edit {
                        (*feedback).result = Some((
                            read_text((*feedback).desc_edit),
                            read_text((*feedback).contact_edit),
                        ));
                        DestroyWindow(hwnd);
                        continue;
                    }
                }
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        window.store(0, Ordering::SeqCst);

        let feedback = Box::from_raw(feedback);
        for font in [
            feedback.font_title,
            feedback.font_lbl,
            feedback.font_edit,
            feedback.font_btn,
        ] {
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        DeleteObject(feedback.bg_brush);
        DeleteObject(feedback.field_brush);
        UnregisterClassW(class_w.as_ptr(), instance);
        feedback.result
    }
}

struct ThreadDpiScope {
    previous: isize,
    restore: Option<unsafe extern "system" fn(isize) -> isize>,
}

impl ThreadDpiScope {
    unsafe fn per_monitor_v2() -> Self {
        unsafe {
            let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
            if user32.is_null() {
                return Self {
                    previous: 0,
                    restore: None,
                };
            }
            type FnSetThread = unsafe extern "system" fn(isize) -> isize;
            match GetProcAddress(user32, c"SetThreadDpiAwarenessContext".as_ptr().cast()) {
                Some(p) => {
                    let f: FnSetThread = std::mem::transmute(p);
                    let previous = f(-4isize);
                    Self {
                        previous,
                        restore: (previous != 0).then_some(f),
                    }
                }
                None => Self {
                    previous: 0,
                    restore: None,
                },
            }
        }
    }
}

impl Drop for ThreadDpiScope {
    fn drop(&mut self) {
        if let Some(f) = self.restore {
            unsafe { f(self.previous) };
        }
    }
}

unsafe fn place_window(hwnd: HWND) {
    unsafe {
        let dpi = dpi_for(hwnd);
        let (target_w, target_h) = (dp(560, dpi), dp(430, dpi));

        let mut rc = std::mem::zeroed::<RECT>();
        let mut wr = std::mem::zeroed::<RECT>();
        GetClientRect(hwnd, &mut rc);
        GetWindowRect(hwnd, &mut wr);
        let chrome_w = (wr.right - wr.left) - (rc.right - rc.left);
        let chrome_h = (wr.bottom - wr.top) - (rc.bottom - rc.top);
        let w = target_w + chrome_w;
        let h = target_h + chrome_h;

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let fg = GetForegroundWindow();
        let mut fr = std::mem::zeroed::<RECT>();
        let (cx, cy) = if !fg.is_null() && GetWindowRect(fg, &mut fr) != 0 {
            ((fr.left + fr.right) / 2, (fr.top + fr.bottom) / 2)
        } else {
            (sw / 2, sh / 2)
        };
        let x = (cx - w / 2).clamp(0, (sw - w).max(0));
        let y = (cy - h / 2).clamp(0, (sh - h).max(0));

        SetWindowPos(hwnd, HWND_TOP, x, y, w, h, 0);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam as *const CREATESTRUCTW);
                let feedback = cs.lpCreateParams as *mut Feedback;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, feedback as isize);
                (*feedback).dpi = dpi_for(hwnd);
                build_controls(hwnd, &mut *feedback);
                0
            }
            WM_SIZE => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if !feedback.is_null() && !(*feedback).desc_edit.is_null() {
                    layout(hwnd, &*feedback);
                    InvalidateRect(hwnd, null_mut(), 1);
                }
                0
            }
            WM_GETMINMAXINFO => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if !feedback.is_null() {
                    let dpi = (*feedback).dpi;
                    let mmi = &mut *(lparam as *mut MINMAXINFO);
                    mmi.ptMinTrackSize.x = dp(420, dpi);
                    mmi.ptMinTrackSize.y = dp(360, dpi);
                }
                0
            }
            WM_DPICHANGED => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if !feedback.is_null() {
                    (*feedback).dpi = (wparam & 0xffff) as u32;
                    apply_fonts(&mut *feedback);
                    let r = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER,
                    );
                }
                0
            }
            WM_ERASEBKGND => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let mut rc = std::mem::zeroed::<RECT>();
                GetClientRect(hwnd, &mut rc);
                FillRect(wparam as HDC, &rc, (*feedback).bg_brush);
                1
            }
            WM_PAINT => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                paint_fields(hwnd, &*feedback);
                0
            }
            WM_CTLCOLORSTATIC => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let hdc = wparam as HDC;
                SetBkMode(hdc, TRANSPARENT as i32);
                let id = GetDlgCtrlID(lparam as HWND) as usize;
                SetTextColor(hdc, if id == ID_TITLE { CLR_TEXT } else { CLR_MUTED });
                (*feedback).bg_brush as LRESULT
            }
            WM_CTLCOLOREDIT => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let hdc = wparam as HDC;
                SetTextColor(hdc, CLR_TEXT);
                SetBkColor(hdc, CLR_FIELD);
                (*feedback).field_brush as LRESULT
            }
            WM_DRAWITEM => {
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                draw_button(&*feedback, &*(lparam as *const DRAWITEMSTRUCT));
                1
            }
            WM_COMMAND => {
                let id = wparam & 0xffff;
                let notify = ((wparam >> 16) & 0xffff) as u32;
                let feedback = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Feedback;
                if feedback.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if notify == EN_SETFOCUS || notify == EN_KILLFOCUS {
                    let ctl = lparam as HWND;
                    if !ctl.is_null()
                        && (ctl == (*feedback).desc_edit || ctl == (*feedback).contact_edit)
                    {
                        if notify == EN_SETFOCUS {
                            (*feedback).focus_edit = ctl;
                        } else if (*feedback).focus_edit == ctl {
                            (*feedback).focus_edit = null_mut();
                        }
                        invalidate_ring(hwnd, &*feedback, ctl);
                    }
                    return 0;
                }
                if id == ID_OK {
                    (*feedback).result = Some((
                        read_text((*feedback).desc_edit),
                        read_text((*feedback).contact_edit),
                    ));
                    DestroyWindow(hwnd);
                } else if id == ID_CANCEL {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn build_controls(hwnd: HWND, feedback: &mut Feedback) {
    unsafe {
        let instance = own_module();

        let title_w = {
            let len = GetWindowTextLengthW(hwnd).max(0) as usize;
            let mut buf = vec![0u16; len + 1];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            buf.truncate(n.max(0) as usize + 1);
            buf
        };
        let static_cls = wide("STATIC");
        let desc_lbl_w = feedback.desc_label.clone();
        let contact_lbl_w = wide(deviceout_i18n::t().prompt_contact);
        feedback.title_lbl = CreateWindowExW(
            0,
            static_cls.as_ptr(),
            title_w.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            ID_TITLE as isize as _,
            instance,
            null_mut(),
        );
        feedback.desc_lbl = CreateWindowExW(
            0,
            static_cls.as_ptr(),
            desc_lbl_w.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            ID_DESC_LBL as isize as _,
            instance,
            null_mut(),
        );
        feedback.contact_lbl = CreateWindowExW(
            0,
            static_cls.as_ptr(),
            contact_lbl_w.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            ID_CONTACT_LBL as isize as _,
            instance,
            null_mut(),
        );

        let edit_cls = wide("EDIT");
        feedback.desc_edit = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            null_mut(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE
                | ES_WANTRETURN
                | ES_AUTOVSCROLL,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        enable_dark_scrollbar(feedback.desc_edit);

        feedback.contact_edit = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            null_mut(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null_mut(),
        );
        SendMessageW(
            feedback.contact_edit,
            EM_SETCUEBANNER,
            1,
            feedback.contact_hint.as_ptr() as LPARAM,
        );

        let btn_cls = wide("BUTTON");
        let ok_l = wide(deviceout_i18n::t().prompt_submit);
        let cancel_l = wide(deviceout_i18n::t().prompt_cancel);
        feedback.ok_btn = CreateWindowExW(
            0,
            btn_cls.as_ptr(),
            ok_l.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32 | BS_DEFPUSHBUTTON as u32,
            0,
            0,
            0,
            0,
            hwnd,
            ID_OK as isize as _,
            instance,
            null_mut(),
        );
        feedback.cancel_btn = CreateWindowExW(
            0,
            btn_cls.as_ptr(),
            cancel_l.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            0,
            0,
            hwnd,
            ID_CANCEL as isize as _,
            instance,
            null_mut(),
        );

        apply_fonts(feedback);
        SendMessageW(hwnd, DM_SETDEFID, ID_OK, 0);
        layout(hwnd, feedback);
    }
}

unsafe fn layout(hwnd: HWND, feedback: &Feedback) {
    unsafe {
        let dpi = feedback.dpi;
        let mut rc = std::mem::zeroed::<RECT>();
        GetClientRect(hwnd, &mut rc);
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;

        let m = dp(MARGIN, dpi);
        let pad = dp(FIELD_PAD, dpi);
        let title_h = dp(TITLE_H, dpi);
        let lbl_h = dp(LBL_H, dpi);
        let bw = dp(BTN_W, dpi);
        let bh = dp(BTN_H, dpi);
        let gap = dp(BTN_GAP, dpi);

        let btn_y = h - m - bh;
        let contact_field_y = btn_y - dp(16, dpi) - dp(CONTACT_FIELD_H, dpi);
        let contact_lbl_y = contact_field_y - dp(LBL_FIELD_GAP, dpi) - lbl_h;
        let desc_lbl_y = m + title_h + dp(10, dpi);
        let desc_field_y = desc_lbl_y + lbl_h + dp(LBL_FIELD_GAP, dpi);
        let desc_field_h = (contact_lbl_y - dp(GROUP_GAP, dpi) - desc_field_y).max(dp(60, dpi));

        MoveWindow(feedback.title_lbl, m, m, w - 2 * m, title_h, 1);
        MoveWindow(feedback.desc_lbl, m, desc_lbl_y, w - 2 * m, lbl_h, 1);
        MoveWindow(
            feedback.desc_edit,
            m + pad,
            desc_field_y + pad,
            w - 2 * m - 2 * pad,
            desc_field_h - 2 * pad,
            1,
        );
        MoveWindow(feedback.contact_lbl, m, contact_lbl_y, w - 2 * m, lbl_h, 1);
        MoveWindow(
            feedback.contact_edit,
            m + pad,
            contact_field_y + pad,
            w - 2 * m - 2 * pad,
            dp(CONTACT_FIELD_H, dpi) - 2 * pad,
            1,
        );
        MoveWindow(feedback.cancel_btn, w - m - bw, btn_y, bw, bh, 1);
        MoveWindow(feedback.ok_btn, w - m - 2 * bw - gap, btn_y, bw, bh, 1);

        let tm = dp(TEXT_MARGIN, dpi) as u32 & 0xffff;
        let margins = (tm | (tm << 16)) as LPARAM;
        for edit in [feedback.desc_edit, feedback.contact_edit] {
            SendMessageW(
                edit,
                EM_SETMARGINS,
                (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
                margins,
            );
        }
    }
}

unsafe fn edit_rect(hwnd: HWND, edit: HWND) -> RECT {
    unsafe {
        let mut rc = std::mem::zeroed::<RECT>();
        GetWindowRect(edit, &mut rc);
        let mut pts = [
            POINT {
                x: rc.left,
                y: rc.top,
            },
            POINT {
                x: rc.right,
                y: rc.bottom,
            },
        ];
        MapWindowPoints(null_mut(), hwnd, pts.as_mut_ptr(), 2);
        RECT {
            left: pts[0].x,
            top: pts[0].y,
            right: pts[1].x,
            bottom: pts[1].y,
        }
    }
}

unsafe fn paint_fields(hwnd: HWND, feedback: &Feedback) {
    unsafe {
        let mut ps = std::mem::zeroed::<PAINTSTRUCT>();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_null() {
            return;
        }
        let pad = dp(FIELD_PAD, feedback.dpi);
        let r = dp(RING_RADIUS, feedback.dpi) * 2;
        for edit in [feedback.desc_edit, feedback.contact_edit] {
            if edit.is_null() {
                continue;
            }
            let mut rc = edit_rect(hwnd, edit);
            InflateRect(&mut rc, pad, pad);
            let ring = if feedback.focus_edit == edit {
                CLR_RING_FOCUS
            } else {
                CLR_RING
            };
            let pen = CreatePen(PS_SOLID, 1, ring);
            let old_pen = SelectObject(hdc, pen);
            let old_brush = SelectObject(hdc, feedback.field_brush);
            RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, r, r);
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            DeleteObject(pen);
        }
        EndPaint(hwnd, &ps);
    }
}

unsafe fn invalidate_ring(hwnd: HWND, feedback: &Feedback, edit: HWND) {
    unsafe {
        let mut rc = edit_rect(hwnd, edit);
        let pad = dp(FIELD_PAD, feedback.dpi) + 2;
        InflateRect(&mut rc, pad, pad);
        InvalidateRect(hwnd, &rc, 1);
    }
}

unsafe fn draw_button(feedback: &Feedback, dis: &DRAWITEMSTRUCT) {
    unsafe {
        if dis.CtlType != ODT_BUTTON {
            return;
        }
        let hdc = dis.hDC;
        let rc = dis.rcItem;
        let pressed = dis.itemState & ODS_SELECTED != 0;
        let is_ok = dis.CtlID as usize == ID_OK;

        FillRect(hdc, &rc, feedback.bg_brush);

        let (fill, text) = if is_ok {
            (
                if pressed {
                    CLR_PRIMARY_DOWN
                } else {
                    CLR_PRIMARY
                },
                CLR_PRIMARY_TEXT,
            )
        } else {
            (if pressed { CLR_SECONDARY_DOWN } else { CLR_BG }, CLR_TEXT)
        };

        let brush = CreateSolidBrush(fill);
        let pen = if is_ok {
            GetStockObject(NULL_PEN) as HPEN
        } else {
            CreatePen(PS_SOLID, 1, CLR_BORDER)
        };
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        let r = dp(6, feedback.dpi);
        RoundRect(
            hdc,
            rc.left + 1,
            rc.top + 1,
            rc.right - 1,
            rc.bottom - 1,
            r * 2,
            r * 2,
        );
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        if !is_ok {
            DeleteObject(pen);
        }

        let mut buf = [0u16; 64];
        let len = GetWindowTextW(dis.hwndItem, buf.as_mut_ptr(), buf.len() as i32);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, text);
        let old_font = SelectObject(hdc, feedback.font_btn);
        let mut trc = rc;
        DrawTextW(
            hdc,
            buf.as_ptr(),
            len,
            &mut trc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
    }
}

unsafe fn create_fonts(feedback: &mut Feedback) {
    unsafe {
        for font in [
            feedback.font_title,
            feedback.font_lbl,
            feedback.font_edit,
            feedback.font_btn,
        ] {
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        feedback.font_title = make_font(feedback.dpi, FONT_TITLE_PX, FW_SEMIBOLD as i32);
        feedback.font_lbl = make_font(feedback.dpi, FONT_LBL_PX, FW_NORMAL as i32);
        feedback.font_edit = make_font(feedback.dpi, FONT_EDIT_PX, FW_NORMAL as i32);
        feedback.font_btn = make_font(feedback.dpi, FONT_BTN_PX, FW_NORMAL as i32);
    }
}

unsafe fn apply_fonts(feedback: &mut Feedback) {
    unsafe {
        create_fonts(feedback);
        for (ctl, font) in [
            (feedback.title_lbl, feedback.font_title),
            (feedback.desc_lbl, feedback.font_lbl),
            (feedback.contact_lbl, feedback.font_lbl),
            (feedback.desc_edit, feedback.font_edit),
            (feedback.contact_edit, feedback.font_edit),
            (feedback.ok_btn, feedback.font_btn),
            (feedback.cancel_btn, feedback.font_btn),
        ] {
            if !ctl.is_null() && !font.is_null() {
                SendMessageW(ctl, WM_SETFONT, font as usize, 1);
            }
        }
    }
}

unsafe fn make_font(dpi: u32, px: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -dp(px, dpi),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_SWISS as u32,
            wide(font_face()).as_ptr(),
        )
    }
}

fn font_face() -> &'static str {
    super::fonts::gdi_face(deviceout_i18n::current().script())
}

pub(crate) fn ensure_noto_gdi() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        for bytes in [
            super::fonts::NOTO_SANS,
            super::fonts::NOTO_THAI,
            super::fonts::NOTO_CJK,
        ] {
            let mut n = 0u32;
            let _ = AddFontMemResourceEx(
                bytes.as_ptr() as *const _,
                bytes.len() as u32,
                std::ptr::null(),
                &mut n,
            );
        }
    });
}

unsafe fn dpi_for(hwnd: HWND) -> u32 {
    unsafe {
        let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
        if !user32.is_null() {
            type FnGetDpi = unsafe extern "system" fn(HWND) -> u32;
            if let Some(p) = GetProcAddress(user32, c"GetDpiForWindow".as_ptr().cast()) {
                let f: FnGetDpi = std::mem::transmute(p);
                let dpi = f(hwnd);
                if dpi != 0 {
                    return dpi;
                }
            }
        }
        96
    }
}

type FnSetAttr = unsafe extern "system" fn(HWND, u32, *const core::ffi::c_void, u32) -> i32;

unsafe fn with_dwm_attr(hwnd: HWND, apply: impl Fn(FnSetAttr, HWND)) {
    unsafe {
        let dwm = LoadLibraryW(wide("dwmapi.dll").as_ptr());
        if dwm.is_null() {
            return;
        }
        if let Some(p) = GetProcAddress(dwm, c"DwmSetWindowAttribute".as_ptr().cast()) {
            let f: FnSetAttr = std::mem::transmute(p);
            apply(f, hwnd);
        }
        FreeLibrary(dwm);
    }
}

unsafe fn enable_dark_title_bar(hwnd: HWND) {
    unsafe {
        with_dwm_attr(hwnd, |f, hwnd| {
            let dark: i32 = 1;
            if f(hwnd, 20, &dark as *const _ as _, 4) != 0 {
                f(hwnd, 19, &dark as *const _ as _, 4);
            }
        });
    }
}

unsafe fn enable_rounded_corners(hwnd: HWND) {
    unsafe {
        with_dwm_attr(hwnd, |f, hwnd| {
            let pref: i32 = 2;
            f(hwnd, 33, &pref as *const _ as _, 4);
        });
    }
}

unsafe fn enable_dark_scrollbar(hwnd: HWND) {
    unsafe {
        let uxtheme = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if uxtheme.is_null() {
            return;
        }
        type FnSetTheme = unsafe extern "system" fn(HWND, *const u16, *const u16) -> i32;
        if let Some(p) = GetProcAddress(uxtheme, c"SetWindowTheme".as_ptr().cast()) {
            let f: FnSetTheme = std::mem::transmute(p);
            f(hwnd, wide("DarkMode_Explorer").as_ptr(), null_mut());
        }
        FreeLibrary(uxtheme);
    }
}

fn read_text(edit: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(edit);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return String::new();
        }
        std::ffi::OsString::from_wide(&buf[..n as usize])
            .to_string_lossy()
            .into_owned()
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}
