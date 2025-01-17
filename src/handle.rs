use std::io::ErrorKind;
use std::mem::size_of;
use std::ops::Deref;

use bitflags::bitflags;
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HMODULE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, CREATE_TOOLHELP_SNAPSHOT_FLAGS,
    MODULEENTRY32W,
};
use windows::Win32::System::Memory::{
    VirtualQueryEx, MEMORY_BASIC_INFORMATION, PAGE_PROTECTION_FLAGS, PAGE_TYPE,
    VIRTUAL_ALLOCATION_TYPE,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS};

use crate::memory::MemoryBasicInformation;
use crate::module::Module;

// TODO: bitflags bad at doc generation
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

/// process handle
pub struct Handle {
    raw: HANDLE,
    process_id: u32,
}

impl Handle {
    /// process id of the handle
    pub fn get_process_id(&self) -> u32 {
        self.process_id
    }

    /// createting handle snapshot
    pub fn create_snapshot(&self, flag: HandleSnapshotFlag) -> Result<HandleSnapshot, ErrorKind> {
        let new_handle = HandleSnapshot {
            raw: unsafe { CreateToolhelp32Snapshot(flag.into(), self.process_id) }
                .map_err(|_| ErrorKind::Other)?,
            process_id: self.process_id,
        };
        return Ok(new_handle);
    }

    /// iterator for memory information related to handle
    pub fn get_memory_basic_informations(&self) -> HandleMemoryBasicInformationIter {
        HandleMemoryBasicInformationIter {
            handle: self,
            current_address: None,
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
        let h = {
            let mut h: HANDLE = HANDLE(0);

            h = unsafe { OpenProcess(PROCESS_ACCESS_RIGHTS(0xFFFF), BOOL(0), value) }
                .map_err(|_| ErrorKind::Other)?;

            if h.is_invalid() {
                h = unsafe { OpenProcess(PROCESS_ACCESS_RIGHTS(0x10 | 0x20), BOOL(0), value) }
                    .map_err(|_| ErrorKind::Other)?;
            }

            if h.is_invalid() {
                return Err(ErrorKind::Other);
            }

            h
        };

        return Ok(Self {
            raw: h,
            process_id: value,
        });
    }
}

/// Process Handle Snapshot
pub struct HandleSnapshot {
    raw: HANDLE,
    process_id: u32,
}

impl HandleSnapshot {
    /// get process id
    pub fn get_process_id(&self) -> u32 {
        self.process_id
    }

    /// get modules
    pub fn get_modules(&self) -> HandleSnapshotModuleIter {
        HandleSnapshotModuleIter {
            handle: self,
            is_first: true,
        }
    }
}

impl Deref for HandleSnapshot {
    type Target = HANDLE;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Drop for HandleSnapshot {
    fn drop(&mut self) {
        if !self.raw.is_invalid() {
            let _ = unsafe { CloseHandle(**self) };
        }
    }
}

/// Process Handle Snapshot -> Module Iterator
pub struct HandleSnapshotModuleIter<'a> {
    handle: &'a HandleSnapshot,
    is_first: bool,
}

impl<'a> Iterator for HandleSnapshotModuleIter<'a> {
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
            match unsafe { Module32FirstW(**self.handle, &mut module_entry_32w as *mut _) } {
                Ok(_) => {
                    self.is_first = false;
                    return Some(Module::from(module_entry_32w));
                }
                Err(_) => {
                    return None;
                }
            }
        }

        match unsafe { Module32NextW(**self.handle, &mut module_entry_32w as *mut _) } {
            Ok(_) => {
                return Some(Module::from(module_entry_32w));
            }
            Err(_) => {
                return None;
            }
        }
    }
}

/// Process Handle -> Memory Basic Information Iterator
pub struct HandleMemoryBasicInformationIter<'a> {
    handle: &'a Handle,
    current_address: Option<usize>,
}

impl<'a> Iterator for HandleMemoryBasicInformationIter<'a> {
    type Item = MemoryBasicInformation;

    fn next(&mut self) -> Option<Self::Item> {
        let mut mbi = MEMORY_BASIC_INFORMATION {
            BaseAddress: std::ptr::null_mut(),
            AllocationBase: std::ptr::null_mut(),
            AllocationProtect: PAGE_PROTECTION_FLAGS(0),
            #[cfg(target_arch = "x86_64")]
            PartitionId: 0,
            RegionSize: 0,
            State: VIRTUAL_ALLOCATION_TYPE(0),
            Protect: PAGE_PROTECTION_FLAGS(0),
            Type: PAGE_TYPE(0),
        };

        let n = unsafe {
            VirtualQueryEx(
                **self.handle,
                self.current_address.map(|v| v as *const _),
                &mut mbi as *mut _,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };

        if n != 0 {
            let mbi = MemoryBasicInformation::from(mbi);

            self.current_address = Some(mbi.get_region_size() + self.current_address.unwrap_or(0));

            return Some(mbi);
        }

        None
    }
}
