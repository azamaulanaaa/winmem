use std::io::ErrorKind;
use std::mem::size_of;
use std::ops::Deref;

use bitflags::bitflags;
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HMODULE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, CREATE_TOOLHELP_SNAPSHOT_FLAGS,
    MODULEENTRY32W,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS};

use crate::module::Module;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct HandleSnapshotFlag: u32 {
        const Inherit = 0x80000000;
        const SnapAll = Self::Inherit.bits() | Self::SnapHeapList.bits() | Self::SnapModule.bits() | Self::SnapModule32.bits() | Self::SnapProcess.bits() | Self::SnapThread.bits();
        const SnapHeapList = 0x1;
        const SnapModule = 0x8;
        const SnapModule32 = 0x10;
        const SnapProcess = 0x2;
        const SnapThread = 0x4;
    }
}

impl Into<CREATE_TOOLHELP_SNAPSHOT_FLAGS> for HandleSnapshotFlag {
    fn into(self) -> CREATE_TOOLHELP_SNAPSHOT_FLAGS {
        CREATE_TOOLHELP_SNAPSHOT_FLAGS(self.bits())
    }
}

#[derive(Debug, Clone)]
pub struct Handle {
    raw: HANDLE,
    process_id: u32,
    is_snapshot: bool,
}

impl Handle {
    pub fn get_process_id(&self) -> u32 {
        self.process_id
    }

    pub fn is_snapshot(&self) -> bool {
        self.is_snapshot
    }

    pub fn create_snapshot(&self, flag: HandleSnapshotFlag) -> Result<Self, ErrorKind> {
        if self.is_snapshot {
            return Err(ErrorKind::InvalidData);
        }

        let new_handle = Handle {
            raw: unsafe { CreateToolhelp32Snapshot(flag.into(), self.process_id) }
                .map_err(|_| ErrorKind::Other)?,
            process_id: self.process_id,
            is_snapshot: true,
        };
        return Ok(new_handle);
    }

    pub fn get_modules(&self) -> HandleModuleIter {
        HandleModuleIter {
            handle: self.raw,
            is_first: true,
        }
    }
}

impl Default for Handle {
    fn default() -> Self {
        Handle::try_from(unsafe { GetCurrentProcessId() }).unwrap()
    }
}

impl Deref for Handle {
    type Target = HANDLE;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.raw.is_invalid() {
            let _ = unsafe { CloseHandle(**self) };
        }
    }
}

impl TryFrom<u32> for Handle {
    type Error = ErrorKind;

    fn try_from(value: u32) -> Result<Handle, Self::Error> {
        let h = unsafe { OpenProcess(PROCESS_ACCESS_RIGHTS(0xFFFF), BOOL(0), value) }
            .map_err(|_| ErrorKind::Other)?;
        if !h.is_invalid() {
            return Ok(Handle {
                raw: h,
                process_id: value,
                is_snapshot: false,
            });
        }

        let h = unsafe { OpenProcess(PROCESS_ACCESS_RIGHTS(0x10 | 0x20), BOOL(0), value) }
            .map_err(|_| ErrorKind::Other)?;
        if !h.is_invalid() {
            return Ok(Handle {
                raw: h,
                process_id: value,
                is_snapshot: false,
            });
        }

        return Err(ErrorKind::Other);
    }
}

pub struct HandleModuleIter {
    handle: HANDLE,
    is_first: bool,
}

impl Iterator for HandleModuleIter {
    type Item = Module;

    fn next(&mut self) -> Option<Self::Item> {
        let mut module_entry_32w = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            GlblcntUsage: 0,
            th32ProcessID: 0,
            th32ModuleID: 0,
            ProccntUsage: 0,
            modBaseAddr: std::ptr::null_mut(),
            modBaseSize: 0,
            hModule: HMODULE(0),
            szModule: [0; 256],
            szExePath: [0; 260],
        };

        if self.is_first {
            match unsafe { Module32FirstW(self.handle, &mut module_entry_32w as *mut _) } {
                Ok(_) => {
                    self.is_first = false;
                    return Some(Module::from(module_entry_32w));
                }
                Err(_) => {
                    return None;
                }
            }
        }

        match unsafe { Module32NextW(self.handle, &mut module_entry_32w as *mut _) } {
            Ok(_) => {
                return Some(Module::from(module_entry_32w));
            }
            Err(_) => {
                return None;
            }
        }
    }
}