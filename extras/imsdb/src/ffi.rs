//! Length-bearing C ABI used by the experimental CBLTDLI shim.
use crate::call_adapter::{CallAdapter, NumericByteOrder, PcbEncoding};
use crate::gsam::GsamSession;
use crate::{Database, Dbd, Psb};
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::path::Path;
use std::sync::Mutex;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImsSpan {
    pub data: *mut u8,
    pub len: usize,
}

pub struct ImsHandle {
    adapter: Mutex<CallAdapter>,
    gsam: Mutex<HashMap<usize, GsamSession>>,
    encoding: PcbEncoding,
    psb: Psb,
}

fn c_string(ptr: *const c_char, name: &str) -> Result<String, String> {
    if ptr.is_null() {
        return Err(format!("{name} is null"));
    }
    // SAFETY: callers must pass a valid NUL-terminated C string.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| format!("{name} is not UTF-8"))
}

unsafe fn report(error: *mut c_char, capacity: usize, message: &str) {
    if error.is_null() || capacity == 0 {
        return;
    }
    let count = message.len().min(capacity - 1);
    // SAFETY: the C caller provides a writable error buffer of `capacity` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(message.as_ptr(), error.cast(), count);
        *error.add(count) = 0;
    }
}

fn load_dbds(dir: &str) -> Result<Vec<Dbd>, String> {
    let mut dbds = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("asm") || ext.eq_ignore_ascii_case("dbd"))
        {
            dbds.push(Dbd::from_file(path)?);
        }
    }
    if dbds.is_empty() {
        return Err("DBD directory contains no .asm or .dbd files".into());
    }
    Ok(dbds)
}

/// Returns null on failure. `encoding` bit 0 selects 1047 EBCDIC control text;
/// bit 1 selects native little-endian COMP-5 at the caller boundary.
/// The returned handle owns an exclusive lock on the WAL until `imsdb_close`.
///
/// # Safety
///
/// Each path must point to a valid NUL-terminated UTF-8 C string. If `error` is
/// non-null, it must be writable for `error_capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn imsdb_open(
    wal: *const c_char,
    dbd_dir: *const c_char,
    psb_file: *const c_char,
    encoding: u32,
    error: *mut c_char,
    error_capacity: usize,
) -> *mut ImsHandle {
    let result = std::panic::catch_unwind(|| -> Result<ImsHandle, String> {
        let wal = c_string(wal, "WAL path")?;
        let dbd_dir = c_string(dbd_dir, "DBD directory")?;
        let psb_file = c_string(psb_file, "PSB file")?;
        if encoding > 3 {
            return Err("unsupported PCB encoding flags".into());
        }
        let text_encoding = if encoding & 1 == 1 {
            PcbEncoding::Cp1047
        } else {
            PcbEncoding::Ascii
        };
        let dbds = load_dbds(&dbd_dir)?;
        let psb = Psb::from_file(&psb_file)?;
        psb.validate(&dbds)?;
        let gsam = psb
            .pcbs
            .iter()
            .enumerate()
            .filter(|(_, pcb)| pcb.kind == "GSAM")
            .map(|(index, pcb)| {
                let name = pcb.dbd_name.as_deref().ok_or("GSAM PCB has no DBDNAME")?;
                let definition = dbds
                    .iter()
                    .find(|dbd| dbd.name == name)
                    .and_then(|dbd| dbd.gsam.clone())
                    .ok_or_else(|| format!("missing GSAM DBD {name}"))?;
                Ok((
                    index,
                    GsamSession::new(name, pcb, definition, text_encoding),
                ))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let database = Database::open(Path::new(&wal), dbds).map_err(|e| e.to_string())?;
        let mut adapter = CallAdapter::new(database, psb.clone(), text_encoding)?;
        if encoding & 2 == 2 {
            adapter.set_numeric_byte_order(NumericByteOrder::Little);
        }
        Ok(ImsHandle {
            adapter: Mutex::new(adapter),
            gsam: Mutex::new(gsam),
            encoding: text_encoding,
            psb,
        })
    });
    match result {
        Ok(Ok(handle)) => Box::into_raw(Box::new(handle)),
        Ok(Err(message)) => {
            unsafe { report(error, error_capacity, &message) };
            std::ptr::null_mut()
        }
        Err(_) => {
            unsafe { report(error, error_capacity, "IMS DB open panicked") };
            std::ptr::null_mut()
        }
    }
}

/// Resolve a unique DB or GSAM PCB by its ASCII DBD name. Returns -1 if absent or ambiguous.
///
/// # Safety
///
/// `handle` must remain open for the call, and `dbd_name` must point to at least
/// `name_len` readable bytes. The caller must not close the handle concurrently.
#[no_mangle]
pub unsafe extern "C" fn imsdb_pcb_index(
    handle: *const ImsHandle,
    dbd_name: *const u8,
    name_len: usize,
) -> i32 {
    if handle.is_null() || dbd_name.is_null() || name_len == 0 || name_len > 8 {
        return -1;
    }
    // SAFETY: callers provide a live handle and a readable name buffer.
    let (handle, name) = unsafe { (&*handle, std::slice::from_raw_parts(dbd_name, name_len)) };
    let Ok(name) = std::str::from_utf8(name) else {
        return -1;
    };
    let name = name.trim().to_ascii_uppercase();
    let mut matches = handle.psb.pcbs.iter().enumerate().filter(|(_, pcb)| {
        matches!(pcb.kind.as_str(), "DB" | "GSAM") && pcb.dbd_name.as_deref() == Some(&name)
    });
    match (matches.next(), matches.next()) {
        (Some((index, _)), None) => i32::try_from(index).unwrap_or(-1),
        _ => -1,
    }
}

unsafe fn copy_input(
    ptr: *const u8,
    len: usize,
    limit: usize,
    name: &str,
) -> Result<Vec<u8>, String> {
    if len > limit || (len > 0 && ptr.is_null()) {
        return Err(format!("invalid {name} pointer or length"));
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: the C caller provides a readable buffer of `len` bytes.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
}

fn overlaps(a: *const u8, a_len: usize, b: *const u8, b_len: usize) -> bool {
    if a_len == 0 || b_len == 0 {
        return false;
    }
    let (a_start, b_start) = (a as usize, b as usize);
    match (a_start.checked_add(a_len), b_start.checked_add(b_len)) {
        (Some(a_end), Some(b_end)) => a_start < b_end && b_start < a_end,
        _ => true,
    }
}

/// Returns 0 for a completed DL/I call (including nonblank PCB status), -1 for an ABI error.
/// Input pointers are copied before mutable PCB and I/O slices are created.
///
/// # Safety
///
/// `handle` must remain open throughout the call. Each non-null input pointer
/// must be readable for its given length. `ssas` must reference `ssa_count`
/// valid descriptors. `pcb` and `io` must be writable for their given lengths
/// and must not overlap. A GSAM RSA span must also be writable and must not
/// overlap either area. If `error` is non-null, it must be writable for
/// `error_capacity` bytes. The caller must not close the handle concurrently.
#[no_mangle]
pub unsafe extern "C" fn imsdb_call(
    handle: *mut ImsHandle,
    pcb_index: u32,
    function: *const u8,
    function_len: usize,
    pcb: *mut u8,
    pcb_len: usize,
    io: *mut u8,
    io_len: usize,
    ssas: *const ImsSpan,
    ssa_count: usize,
    error: *mut c_char,
    error_capacity: usize,
) -> i32 {
    let result = std::panic::catch_unwind(|| -> Result<(), String> {
        if handle.is_null()
            || pcb.is_null()
            || io.is_null()
            || pcb_len > 4096
            || io_len > 64 * 1024 * 1024
        {
            return Err("invalid handle, PCB, or I/O area".into());
        }
        if overlaps(pcb, pcb_len, io, io_len) {
            return Err("PCB and I/O area overlap".into());
        }
        let function = unsafe { copy_input(function, function_len, 4, "function") }?;
        if ssa_count > 16 || (ssa_count > 0 && ssas.is_null()) {
            return Err("invalid SSA array".into());
        }
        let handle = unsafe { &*handle };
        if handle
            .psb
            .pcbs
            .get(pcb_index as usize)
            .is_some_and(|pcb| pcb.kind == "GSAM")
        {
            if ssa_count > 1 {
                return Err("GSAM accepts at most one RSA".into());
            }
            let span = if ssa_count == 1 {
                Some(unsafe { ssas.read_unaligned() })
            } else {
                None
            };
            if let Some(span) = span {
                if span.data.is_null()
                    || span.len != 8
                    || overlaps(span.data, span.len, pcb, pcb_len)
                    || overlaps(span.data, span.len, io, io_len)
                {
                    return Err("invalid or overlapping GSAM RSA".into());
                }
            }
            let function: Vec<u8> = function
                .iter()
                .map(|byte| handle.encoding.decode_control(*byte))
                .collect();
            let function = std::str::from_utf8(&function).map_err(|_| "invalid GSAM function")?;
            // SAFETY: the caller provides writable, pairwise non-overlapping areas.
            let pcb = unsafe { std::slice::from_raw_parts_mut(pcb, pcb_len) };
            let io = unsafe { std::slice::from_raw_parts_mut(io, io_len) };
            let rsa =
                span.map(|span| unsafe { std::slice::from_raw_parts_mut(span.data, span.len) });
            let mut sessions = handle
                .gsam
                .lock()
                .map_err(|_| "GSAM session lock poisoned")?;
            return sessions
                .get_mut(&(pcb_index as usize))
                .ok_or("missing GSAM session")?
                .call(function, pcb, io, rsa);
        }
        let mut owned_ssas = Vec::with_capacity(ssa_count);
        for index in 0..ssa_count {
            // SAFETY: the caller supplies an array of `ssa_count` descriptors.
            let span = unsafe { ssas.add(index).read_unaligned() };
            owned_ssas.push(unsafe { copy_input(span.data, span.len, 4096, "SSA") }?);
        }
        let refs: Vec<&[u8]> = owned_ssas.iter().map(Vec::as_slice).collect();
        // SAFETY: the C caller provides writable non-overlapping PCB and I/O buffers.
        let (pcb, io) = unsafe {
            (
                std::slice::from_raw_parts_mut(pcb, pcb_len),
                std::slice::from_raw_parts_mut(io, io_len),
            )
        };
        let mut adapter = handle
            .adapter
            .lock()
            .map_err(|_| "IMS DB session lock poisoned")?;
        adapter.call_encoded(pcb_index as usize, &function, pcb, io, &refs)?;
        Ok(())
    });
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(message)) => {
            unsafe { report(error, error_capacity, &message) };
            -1
        }
        Err(_) => {
            unsafe { report(error, error_capacity, "IMS DB call panicked") };
            -1
        }
    }
}

/// Commit the current unit of work.
///
/// # Safety
///
/// `handle` must remain open throughout the call. If `error` is non-null, it
/// must be writable for `error_capacity` bytes. Do not close the handle
/// concurrently.
#[no_mangle]
pub unsafe extern "C" fn imsdb_commit(
    handle: *mut ImsHandle,
    error: *mut c_char,
    error_capacity: usize,
) -> i32 {
    if handle.is_null() {
        unsafe { report(error, error_capacity, "IMS DB handle is null") };
        return -1;
    }
    let result = std::panic::catch_unwind(|| -> Result<(), String> {
        let handle = unsafe { &*handle };
        handle
            .adapter
            .lock()
            .map_err(|_| "IMS DB session lock poisoned".to_owned())?
            .commit()?;
        for session in handle
            .gsam
            .lock()
            .map_err(|_| "GSAM session lock poisoned".to_owned())?
            .values_mut()
        {
            session.commit()?;
        }
        Ok(())
    });
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(message)) => {
            unsafe { report(error, error_capacity, &message) };
            -1
        }
        Err(_) => {
            unsafe { report(error, error_capacity, "IMS DB commit panicked") };
            -1
        }
    }
}

/// Commit the current UOW with an eight-byte basic checkpoint ID.
///
/// # Safety
///
/// `handle` must remain open, `id` must point to eight readable bytes, and
/// `error` must be writable for `error_capacity` bytes when non-null.
#[no_mangle]
pub unsafe extern "C" fn imsdb_checkpoint(
    handle: *mut ImsHandle,
    id: *const u8,
    error: *mut c_char,
    error_capacity: usize,
) -> i32 {
    if handle.is_null() || id.is_null() {
        unsafe {
            report(
                error,
                error_capacity,
                "IMS DB handle or checkpoint ID is null",
            )
        };
        return -1;
    }
    let checkpoint = unsafe { std::slice::from_raw_parts(id, 8) };
    let checkpoint: [u8; 8] = checkpoint.try_into().unwrap();
    let result = std::panic::catch_unwind(|| -> Result<(), String> {
        let handle = unsafe { &*handle };
        handle
            .adapter
            .lock()
            .map_err(|_| "IMS DB session lock poisoned".to_owned())?
            .commit_checkpoint(Some(checkpoint))?;
        for session in handle
            .gsam
            .lock()
            .map_err(|_| "GSAM session lock poisoned".to_owned())?
            .values_mut()
        {
            session.commit()?;
        }
        Ok(())
    });
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(message)) => {
            unsafe { report(error, error_capacity, &message) };
            -1
        }
        Err(_) => {
            unsafe { report(error, error_capacity, "IMS DB checkpoint panicked") };
            -1
        }
    }
}

/// Discard the current unit of work.
///
/// # Safety
///
/// `handle` must remain open throughout the call. Do not close it concurrently.
#[no_mangle]
pub unsafe extern "C" fn imsdb_rollback(handle: *mut ImsHandle) {
    if !handle.is_null() {
        let _ = std::panic::catch_unwind(|| {
            if let Ok(mut adapter) = unsafe { &*handle }.adapter.lock() {
                adapter.rollback();
            }
            if let Ok(mut sessions) = unsafe { &*handle }.gsam.lock() {
                for session in sessions.values_mut() {
                    session.rollback();
                }
            }
        });
    }
}

/// Release the handle and its WAL lock.
///
/// # Safety
///
/// `handle` must have come from `imsdb_open` and must be closed at most once.
/// No other thread may use it concurrently with this call.
#[no_mangle]
pub unsafe extern "C" fn imsdb_close(handle: *mut ImsHandle) {
    if !handle.is_null() {
        // SAFETY: `handle` was returned by imsdb_open and is closed once.
        drop(unsafe { Box::from_raw(handle) });
    }
}
