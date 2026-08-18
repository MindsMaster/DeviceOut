use windows::core::w;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};

use crate::error::SinkError;

#[derive(Debug)]
pub struct AudioPriority {
    task: HANDLE,
}

impl AudioPriority {
    pub fn raise_current_thread() -> Result<Self, SinkError> {
        let mut task_index = 0u32;
        let task = unsafe { AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index) }
            .map_err(|e| SinkError::Stream(format!("注册 MMCSS 音频优先级失败: {e}")))?;

        Ok(Self { task })
    }
}

impl Drop for AudioPriority {
    fn drop(&mut self) {
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(self.task);
        }
    }
}
