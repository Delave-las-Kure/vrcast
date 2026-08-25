//! Проверка, что интеграционная оснастка сама за собой убирает.
//!
//! Оснастка запускает контейнеры, и если она их не удаляет, после неудачного прогона
//! в системе копятся висящие серверы. Это ровно тот же класс ошибки, что осиротевший
//! процесс кодирования, — только этажом выше.

use super::fixture::{docker_available, TestServer};

#[test]
fn контейнер_удаляется_вместе_с_тестом() {
    assert!(
        docker_available(),
        "Docker не запущен — интеграционные тесты идти не могут"
    );

    let id_before = std::process::Command::new("docker")
        .args(["ps", "-q"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);

    {
        let server = TestServer::start().expect("контейнер не поднялся");
        let running = std::process::Command::new("docker")
            .args(["ps", "-q"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        assert!(
            running > id_before,
            "контейнер не появился среди работающих"
        );
        drop(server);
    }

    // Даём Docker миг на уборку.
    std::thread::sleep(std::time::Duration::from_secs(2));

    let after = std::process::Command::new("docker")
        .args(["ps", "-q"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);

    assert_eq!(after, id_before, "после теста остался висящий контейнер");
}
