//! Windows Job Object: 서버 프로세스 트리를 러너 수명에 묶는다.
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`로 만든 잡에 서버를 넣고 핸들을 러너가 끝날 때까지 쥔다.
//! 러너가 정상 종료하든 작업 관리자에서 강제 종료되든 핸들이 닫히면 잡 안의 프로세스
//! (서버가 띄운 손자 포함)가 모두 끝난다.
//!
//! 메모리 상한(`--memory-limit`)을 주면 잡 전체의 커밋 메모리를 `JOB_OBJECT_LIMIT_JOB_MEMORY`로 막는다.
//! 넘으면 할당이 실패하고 잡이 완료 포트로 알린다. 그때 잡을 끝내고 그 사실을 남긴다(종료 이유).

use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectAssociateCompletionPortInformation, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED};
use windows_sys::Win32::System::Threading::{OpenProcess, INFINITE, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// 완료 포트 메시지 (winnt.h): 잡 안 프로세스가 모두 끝남, 잡 메모리 상한을 넘음.
/// 메시지는 들어온 차례로 오므로 상한 알림은 "모두 끝남"보다 먼저 온다
const JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO: u32 = 4;
const JOB_OBJECT_MSG_JOB_MEMORY_LIMIT: u32 = 10;
/// 서버가 끝난 뒤 남은 알림을 기다리는 최대 시간
const DRAIN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// 상한을 넘어 잡을 끝낼 때의 종료 코드 (`ERROR_NOT_ENOUGH_MEMORY`)
const MEMORY_LIMIT_EXIT_CODE: u32 = 8;

pub struct Job {
    handle: HANDLE,
    /// 메모리 상한 알림을 받는 완료 포트 (상한이 없으면 null)
    port: HANDLE,
    memory_hit: Arc<AtomicBool>,
    /// 감시 스레드가 "모두 끝남"까지 읽었다 (그 앞의 상한 알림도 처리됐다)
    drained: Arc<AtomicBool>,
}

// 잡 핸들은 스레드 사이에 옮겨도 된다
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// 잡을 만들고 `pid`를 넣는다. 서버가 그 사이 띄운 자식은 빠질 수 있지만,
    /// 넣은 뒤에 띄우는 프로세스는 모두 같은 잡에 들어간다.
    /// `memory_limit`(바이트)를 주면 잡 전체의 커밋 메모리를 그만큼으로 막는다.
    pub fn contain(pid: u32, memory_limit: Option<u64>) -> Result<Self, String> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(format!("CreateJobObject failed ({})", std::io::Error::last_os_error()));
            }
            let mut job = Job {
                handle,
                port: std::ptr::null_mut(),
                memory_hit: Arc::new(AtomicBool::new(false)),
                drained: Arc::new(AtomicBool::new(false)),
            };
            if memory_limit.is_some() {
                job.watch_memory()?;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(bytes) = memory_limit {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = bytes as usize;
            }
            if SetInformationJobObject(
                job.handle,
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
            let assigned = AssignProcessToJobObject(job.handle, process);
            let err = std::io::Error::last_os_error();
            CloseHandle(process);
            if assigned == 0 {
                return Err(format!("AssignProcessToJobObject failed ({})", err));
            }
            Ok(job)
        }
    }

    /// 완료 포트를 잡에 잇고, 메모리 상한 알림이 오면 잡을 끝내는 스레드를 띄운다.
    /// 할당이 실패한 서버는 멈춰 있거나 엉뚱하게 돌 수 있으므로 기다리지 않고 끝낸다
    unsafe fn watch_memory(&mut self) -> Result<(), String> {
        let port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1);
        if port.is_null() {
            return Err(format!("CreateIoCompletionPort failed ({})", std::io::Error::last_os_error()));
        }
        self.port = port;
        let assoc = JOBOBJECT_ASSOCIATE_COMPLETION_PORT { CompletionKey: std::ptr::null_mut(), CompletionPort: port };
        if SetInformationJobObject(
            self.handle,
            JobObjectAssociateCompletionPortInformation,
            &assoc as *const _ as *const _,
            size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
        ) == 0
        {
            return Err(format!("associate completion port failed ({})", std::io::Error::last_os_error()));
        }
        // 핸들은 스레드로 숫자로 넘긴다. 포트를 닫으면(Drop) 기다림이 실패해 스레드가 끝난다
        let (job, port, hit, drained) = (self.handle as usize, port as usize, self.memory_hit.clone(), self.drained.clone());
        std::thread::spawn(move || loop {
            let (mut msg, mut key, mut overlapped): (u32, usize, *mut OVERLAPPED) = (0, 0, std::ptr::null_mut());
            if GetQueuedCompletionStatus(port as HANDLE, &mut msg, &mut key, &mut overlapped, INFINITE) == 0 {
                drained.store(true, Ordering::SeqCst);
                return;
            }
            match msg {
                JOB_OBJECT_MSG_JOB_MEMORY_LIMIT => {
                    hit.store(true, Ordering::SeqCst);
                    TerminateJobObject(job as HANDLE, MEMORY_LIMIT_EXIT_CODE);
                }
                JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO => {
                    drained.store(true, Ordering::SeqCst);
                    return;
                }
                _ => {}
            }
        });
        Ok(())
    }

    /// 메모리 상한을 넘어 잡을 끝냈는가. 서버가 끝난 뒤에 부른다: 서버가 상한에 걸려 곧바로 끝나면
    /// 알림이 아직 큐에 있을 수 있으므로 감시 스레드가 "모두 끝남"까지 읽기를 잠깐 기다린다
    pub fn memory_limit_hit(&self) -> bool {
        let deadline = std::time::Instant::now() + DRAIN_WAIT;
        while !self.port.is_null() && !self.drained.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.memory_hit.load(Ordering::SeqCst)
    }

    /// 잡 안에서 아직 살아 있는 프로세스 수
    pub fn active_processes(&self) -> u32 {
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            let ok = QueryInformationJobObject(
                self.handle,
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
                self.handle,
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
            if !self.port.is_null() {
                CloseHandle(self.port);
            }
            CloseHandle(self.handle);
        }
    }
}
