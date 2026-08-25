//! Тесты уборки уцелевших программ при запуске.
//!
//! Проверяется два противоположных свойства, и второе важнее первого:
//!
//! 1. Уцелевшая от прошлого запуска программа **будет** завершена.
//! 2. Посторонняя программа, занявшая переиспользованный номер, **не будет** тронута.
//!
//! Второе важнее, потому что ошибка здесь дороже: не завершить своё — это осиротевший
//! процесс до следующего запуска, а завершить чужое — это убитый браузер пользователя
//! или, хуже, чужая долгая работа.

use std::time::Duration;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::process::ManagedProcess;
use vrcast_studio_lib::tasks::registry;

fn long_running() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
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
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    } else {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

#[tokio::test]
async fn уцелевшая_программа_добивается_при_запуске() {
    let db = Db::open_in_memory().unwrap();

    // Изображаем предыдущий запуск: программа работает, запись о ней есть,
    // а приложение «умерло», не успев её завершить.
    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alive(pid), "процесс не запустился — проверять нечего");

    registry::record(&db, pid, prog, None).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert!(
        report.killed.contains(&pid),
        "уцелевшая программа не добита: {report:?}"
    );
    assert!(!alive(pid), "процесс {pid} пережил уборку");
    assert!(
        !report.is_clean(),
        "уборка отчиталась, что убирать было нечего"
    );

    // Таблица очищается: всё, что в ней было, относилось к прошлому запуску.
    let second = registry::sweep_on_startup(&db).unwrap();
    assert!(
        second.is_clean(),
        "записи остались после уборки: {second:?}"
    );

    let _ = p.kill_tree().await;
}

#[tokio::test]
async fn посторонняя_программа_по_чужому_номеру_не_трогается() {
    // Главная проверка. Номера процессов переиспользуются: к следующему запуску за старым
    // номером вполне может стоять браузер пользователя. Убивать его недопустимо.
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alive(pid));

    // Запись утверждает, что под этим номером был ffmpeg, — а там на самом деле другое.
    registry::record(&db, pid, "ffmpeg", None).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert!(
        report.killed.is_empty(),
        "УБИТА ПОСТОРОННЯЯ ПРОГРАММА по переиспользованному номеру: {report:?}"
    );
    assert!(
        report.reused.contains(&pid),
        "несовпадение имени не замечено: {report:?}"
    );
    assert!(alive(pid), "процесс {pid} убит, хотя имя не совпало");

    p.kill_tree().await.unwrap();
}

#[tokio::test]
async fn запись_об_уже_завершившейся_программе_безобидна() {
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    registry::record(&db, pid, prog, None).unwrap();

    // Программа завершилась штатно ещё до уборки.
    p.kill_tree().await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let report = registry::sweep_on_startup(&db).unwrap();
    // Запись обязана быть классифицирована, а не молча выброшена: обычно «уже нет»,
    // а при мгновенном переиспользовании номера системой — «номер занят другим».
    // (Раньше правым плечом «или» стояло killed.is_empty() — та же проверка, что
    // и assert ниже, и первое утверждение не могло упасть вовсе.)
    assert!(
        report.already_gone.contains(&pid) || report.reused.contains(&pid),
        "завершившаяся программа обработана неверно: {report:?}"
    );
    assert!(
        report.killed.is_empty(),
        "убито то, чего уже не было: {report:?}"
    );
}

#[tokio::test]
async fn штатное_завершение_убирает_запись() {
    let db = Db::open_in_memory().unwrap();

    let (prog, args) = long_running();
    let mut p = ManagedProcess::spawn(prog, &args).unwrap();
    let pid = p.id().unwrap();
    registry::record(&db, pid, prog, None).unwrap();

    p.kill_tree().await.unwrap();
    registry::forget(&db, pid).unwrap();

    let report = registry::sweep_on_startup(&db).unwrap();
    assert!(
        report.is_clean() && report.already_gone.is_empty(),
        "запись не убрана при штатном завершении: {report:?}"
    );
}
