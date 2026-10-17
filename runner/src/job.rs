//! Windows Job Object: 서버 프로세스 트리를 러너 수명에 묶는다.
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`로 만든 잡에 서버를 넣고 핸들을 러너가 끝날 때까지 쥔다.
//! 러너가 정상 종료하든 작업 관리자에서 강제 종료되든 핸들이 닫히면 잡 안의 프로세스
//! (서버가 띄운 손자 포함)가 모두 끝난다.

use std::mem::{size_of, zeroed};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

pub struct Job(HANDLE);

// 잡 핸들은 스레드 사이에 옮겨도 된다
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// 잡을 만들고 `pid`를 넣는다. 서버가 그 사이 띄운 자식은 빠질 수 있지만,
    /// 넣은 뒤에 띄우는 프로세스는 모두 같은 잡에 들어간다.
    pub fn contain(pid: u32) -> Result<Self, String> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(format!("CreateJobObject failed ({})", std::io::Error::last_os_error()));
            }
            let job = Job(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(format!("SetInformationJobObject failed ({})", std::io::Error::last_os_error()));
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                return Err(format!("OpenProcess({}) failed ({})", pid, std::io::Error::last_os_error()));
            }
            let assigned = AssignProcessToJobObject(job.0, process);
            let err = std::io::Error::last_os_error();
            CloseHandle(process);
            if assigned == 0 {
                return Err(format!("AssignProcessToJobObject failed ({})", err));
            }
            Ok(job)
        }
    }

    /// 잡 안에서 아직 살아 있는 프로세스 수
    pub fn active_processes(&self) -> u32 {
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            let ok = QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                0
            } else {
                info.ActiveProcesses
            }
        }
    }
}

impl Job {
    /// 잡 안에서 살아 있는 프로세스 PID (런처가 남긴 진짜 서버를 찾는 데 쓴다)
    pub fn process_ids(&self) -> Vec<u32> {
        const MAX: usize = 256;
        // 헤더(u32 두 개) 뒤에 PID 배열이 이어지는 가변 길이 구조체
        let mut buf = vec![0usize; 1 + MAX];
        unsafe {
            let ok = QueryInformationJobObject(
                self.0,
                JobObjectBasicProcessIdList,
                buf.as_mut_ptr() as *mut _,
                (buf.len() * size_of::<usize>()) as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                return Vec::new();
            }
            let header = buf.as_ptr() as *const u32;
            let listed = (*header.add(1) as usize).min(MAX);
            buf[1..1 + listed].iter().map(|&pid| pid as u32).collect()
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
