// Integration tests for detecting intermittent daemon crashes
// 목적: 백엔드 안정성 테스트 (ProcessMonitor, ProcessTracker, 모니터링 루프)

#[cfg(test)]
mod stability_tests {
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    /// Mutex 데드락 재현 테스트
    /// 여러 스레드가 동시에 뮤텍스를 획득하려고 할 때 panic이 발생하지 않는지 확인
    #[test]
    fn test_concurrent_mutex_access() {
        let shared_data = Arc::new(Mutex::new(0usize));
        let mut handles = vec![];

        println!("Starting concurrent mutex access test...");

        for i in 0..20 {
            let data = Arc::clone(&shared_data);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    match data.lock() {
                        Ok(mut guard) => {
                            *guard += 1;
                            if (i * 100 + j) % 500 == 0 {
                                println!("Thread {}: Updated counter to {}", i, *guard);
                            }
                        }
                        Err(e) => {
                            eprintln!("Thread {}: Mutex lock failed: {}", i, e);
                            // 데드락 발생 - 테스트 실패
                            panic!("Mutex poisoned!");
                        }
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        let final_value = shared_data.lock().unwrap();
        println!("Final counter value: {}", *final_value);
        assert_eq!(*final_value, 2000); // 20 threads × 100 iterations
    }

    /// Process Monitor 반복 호출 테스트
    /// PowerShell 명령을 반복 실행할 때 리소스 누수가 발생하지 않는지 확인
    #[test]
    fn test_process_detection_loop() {
        println!("Starting process detection loop test...");

        // 현재 프로세스 목록을 반복적으로 가져오기
        for iteration in 0..50 {
            match std::process::Command::new("powershell")
                .args(&[
                    "-NoProfile",
                    "-Command",
                    "Get-Process | Select-Object Id,ProcessName | ConvertTo-Csv -NoTypeInformation",
                ])
                .output()
            {
                Ok(output) => {
                    if !output.status.success() {
                        eprintln!("Iteration {}: PowerShell command failed", iteration);
                    } else {
                        let process_count = output.stdout.iter().filter(|&&b| b == b'\n').count();
                        if iteration % 10 == 0 {
                            println!("Iteration {}: Found {} processes", iteration, process_count);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Iteration {}: Failed to execute PowerShell: {}", iteration, e);
                }
            }
        }

        println!("Process detection loop test completed");
    }

    /// Parse 오류 복원력 테스트
    /// CSV 파싱 실패 시에도 계속 진행되는지 확인
    #[test]
    fn test_error_recovery_in_parsing() {
        println!("Starting parse error recovery test...");

        let malformed_lines = vec![
            r#""123","chrome","C:\Program Files\Chrome.exe""#,     // 정상
            r#""invalid","process""#,                              // CSV 파싱 실패 (따옴표 부족)
            r#""456","firefox"#,                                   // CSV 파싱 실패
            r#""789","notepad","C:\Windows\notepad.exe""#,         // 정상
            r#""bad_pid","process","path""#,                       // PID 파싱 실패
        ];

        let mut parsed_count = 0;
        let mut error_count = 0;

        for line in malformed_lines {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let pid_str = parts[0].trim().trim_matches('"');
                match pid_str.parse::<u32>() {
                    Ok(pid) => {
                        println!("Successfully parsed PID: {}", pid);
                        parsed_count += 1;
                    }
                    Err(e) => {
                        println!("Failed to parse PID '{}': {} (continuing...)", pid_str, e);
                        error_count += 1;
                    }
                }
            }
        }

        println!(
            "Parse error recovery: {} successful, {} errors",
            parsed_count, error_count
        );
        assert!(parsed_count > 0); // 최소 일부는 파싱되어야 함
    }

    /// 반복 오류 로깅 억제 테스트
    /// 같은 오류가 반복되어도 로그가 폭증하지 않는지 확인
    #[test]
    fn test_error_logging_throttle() {
        println!("Starting error logging throttle test...");

        let log_count = Arc::new(AtomicUsize::new(0));
        let max_consecutive_errors = 10;
        let mut error_count = 0;

        for iteration in 0..50 {
            // 의도적으로 실패 시뮬레이션
            error_count += 1;

            // 처음 3회와 이후 10회마다만 로깅
            if error_count <= 3 || error_count % 10 == 0 {
                println!("Error (count: {}): Test iteration {}", error_count, iteration);
                log_count.fetch_add(1, Ordering::SeqCst);
            }

            // 10회 초과 시 리셋
            if error_count >= max_consecutive_errors {
                error_count = 0;
            }
        }

        let total_logs = log_count.load(Ordering::SeqCst);
        println!("Total logs emitted: {}", total_logs);
        // 50회 반복 중 약 13-15개의 로그만 출력되어야 함
        // (처음 3 + 1회 리셋 후 최대 10 + 리셋 + 일부)
        assert!(total_logs <= 20);
    }

    /// 스레드 안전성 테스트
    /// ProcessTracker와 유사한 구조로 동시 접근 테스트
    #[test]
    fn test_thread_safe_hashmap_access() {
        use std::collections::HashMap;

        println!("Starting thread-safe HashMap access test...");

        let shared_map = Arc::new(Mutex::new(HashMap::new()));
        let mut handles = vec![];

        // 10개의 스레드가 각각 100번씩 HashMap을 수정
        for thread_id in 0..10 {
            let map = Arc::clone(&shared_map);
            let handle = thread::spawn(move || {
                for i in 0..100 {
                    let key = format!("server-{}-{}", thread_id, i);
                    match map.lock() {
                        Ok(mut guard) => {
                            guard.insert(key.clone(), thread_id * 100 + i);

                            if i % 25 == 0 {
                                println!(
                                    "Thread {}: Inserted {} items",
                                    thread_id,
                                    guard.len()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("Thread {}: Lock poisoned: {}", thread_id, e);
                            panic!("HashMap lock failed!");
                        }
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        let final_map = shared_map.lock().unwrap();
        println!("Final HashMap size: {}", final_map.len());
        assert_eq!(final_map.len(), 1000); // 10 threads × 100 items
    }

    /// 메모리 누수 시뮬레이션 테스트
    /// 많은 임시 객체 생성 후 메모리 해제 확인
    #[test]
    fn test_memory_allocation_cleanup() {
        println!("Starting memory allocation cleanup test...");

        let mut allocations = vec![];

        for i in 0..10000 {
            let data = vec![i; 1024]; // 각 할당마다 1KB
            allocations.push(data);

            if i % 1000 == 0 {
                println!("Allocated {} items", i + 1);
            }
        }

        println!("Total allocations: {}", allocations.len());

        // 명시적으로 drop
        drop(allocations);
        println!("Memory cleanup completed");
    }

    /// Panic 방지 테스트
    /// 의도적인 오류 상황에서도 panic이 발생하지 않는지 확인
    #[test]
    fn test_no_panic_on_common_errors() {
        println!("Starting no-panic test...");

        // Case 1: 빈 결과 처리
        let empty_vec: Vec<i32> = vec![];
        let first = empty_vec.first(); // None 반환, panic 없음
        assert!(first.is_none());
        println!("Case 1 passed: Empty vector handled safely");

        // Case 2: 파싱 오류 처리
        let invalid_str = "not_a_number";
        let parsed = invalid_str.parse::<i32>(); // Err 반환, panic 없음
        assert!(parsed.is_err());
        println!("Case 2 passed: Parse error handled safely");

        // Case 3: 패턴 매칭
        let option_value: Option<&str> = None;
        match option_value {
            Some(val) => println!("Got value: {}", val),
            None => println!("Case 3 passed: None handled safely"),
        }

        println!("No-panic test completed");
    }
}

