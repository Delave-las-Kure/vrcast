//! T021 — проверка того, что дерево процессов действительно завершается.
//!
//! Конституция, принцип III (НЕОБСУЖДАЕМО) и SC-010. Проверяется не то, что вызов
//! завершения возвращает успех, а то, что **процессов не осталось**: именно осиротевший
//! `ffmpeg`, продолжающий писать в файл результата, и был исходным происшествием.
//!
//! Внук здесь не для полноты картины. `ffmpeg` и `ssh` порождают собственных потомков,
//! и завершение только прямого потомка оставляет их работать — то есть ровно та ошибка,
//! от которой защищаемся.

use std::time::Duration;
use vrcast_studio_lib::tasks::process::ManagedProcess;

/// Долгая команда, доступная на обеих целевых ОС.
fn long_running() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        // ping с большим числом попыток — самый переносимый «спящий» процесс в Windows.
        (
            "cmd",
            vec!["/c".into(), "ping -n 300 127.0.0.1 >nul".into()],
        )
    } else {
        ("sh", vec!["-c".into(), "sleep 300".into()])
    }
}

fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match out {
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stdout);
                text.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    } else {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Идентификаторы прямых потомков указанного процесса.
///
/// Считать процессы по имени нельзя: тесты идут параллельно, и чужие потомки попадают
/// в счёт. Проверять надо родство, а не совпадение имени.
fn children_of(pid: u32) -> Vec<u32> {
    let out = if cfg!(windows) {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}').ProcessId"
                ),
            ])
            .output()
    } else {
        std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
    };

    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[tokio::test]
async fn отмена_завершает_запущенный_процесс() {
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).expect("процесс не запустился");
    let pid = p.id().expect("нет идентификатора процесса");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        alive(pid),
        "процесс не запустился или сразу умер — проверять нечего"
    );

    p.kill_tree().await.expect("завершение не удалось");
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(!alive(pid), "процесс {pid} пережил отмену");
}

#[tokio::test]
async fn отмена_забирает_и_внуков() {
    // Главная проверка. Именно здесь ломается обычное завершение по идентификатору:
    // прямой потомок умирает, а его собственные потомки продолжают работать — ровно как
    // осиротевший ffmpeg, продолжающий портить файл результата.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).expect("процесс не запустился");
    let parent = p.id().expect("нет идентификатора");

    tokio::time::sleep(Duration::from_millis(900)).await;

    let grandchildren = children_of(parent);
    assert!(
        !grandchildren.is_empty(),
        "внуков не появилось — тест ничего не проверяет (родитель {parent})"
    );

    p.kill_tree().await.expect("завершение не удалось");
    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(!alive(parent), "родитель {parent} пережил отмену");
    let survivors: Vec<u32> = grandchildren
        .iter()
        .copied()
        .filter(|g| alive(*g))
        .collect();
    assert!(
        survivors.is_empty(),
        "ОСИРОТЕВШИЕ ПРОЦЕССЫ пережили отмену: {survivors:?} (внуки {grandchildren:?})"
    );
}

#[tokio::test]
async fn повторное_завершение_не_ошибка() {
    // Конституция, принцип V: повтор обязан быть безопасным. Отмену могут нажать дважды,
    // и второй раз не должен превращаться в ошибку.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    p.kill_tree().await.expect("первое завершение");
    p.kill_tree()
        .await
        .expect("повторное завершение не должно быть ошибкой");
}

#[cfg(windows)]
#[tokio::test]
async fn смерть_приложения_забирает_потомков() {
    // Свойство, которого нет у обычного завершения: описатель объекта задания закрывает
    // ядро при смерти процесса-владельца — в том числе когда приложение убито диспетчером
    // задач и ни одна строчка его кода уже не выполняется (SC-010).
    //
    // Здесь это воспроизводится закрытием описателя: роняем структуру, не вызывая
    // завершение явно.
    let (prog, args) = long_running();
    let pid = {
        let p = ManagedProcess::spawn(prog, &args).unwrap();
        let pid = p.id().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(alive(pid), "процесс не запустился");
        pid
        // p роняется здесь: описатель задания закрывается
    };

    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        !alive(pid),
        "процесс {pid} пережил закрытие описателя задания — гарантия не работает"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn приостановка_и_продолжение_работают() {
    // FR-083a. На Unix проверяется по состоянию процесса; на Windows состояние потоков
    // так просто не прочитать, поэтому там это покрывается ручной проверкой.
    let (prog, args) = long_running();
    let p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    p.suspend().expect("приостановка не удалась");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(
        state.contains(") T "),
        "процесс не приостановлен, состояние: {state}"
    );

    p.resume().expect("продолжение не удалось");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(
        !state.contains(") T "),
        "процесс не продолжил работу, состояние: {state}"
    );

    let mut p = p;
    p.kill_tree().await.unwrap();
}
