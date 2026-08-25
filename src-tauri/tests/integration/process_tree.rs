//! Проверка, что интеграционная оснастка сама за собой убирает.
//!
//! Оснастка запускает контейнеры, и если она их не удаляет, после неудачного прогона
//! в системе копятся висящие серверы. Это ровно тот же класс ошибки, что осиротевший
//! процесс кодирования, — только этажом выше.

use super::fixture::{docker_available, TestServer, IMAGE};

/// Сколько НАШИХ контейнеров сейчас работает.
///
/// Считать `docker ps -q` целиком нельзя: демон общий, и посторонний контейнер,
/// стартовавший между двумя замерами, ронял бы тест ни за что. Сбой самого
/// подсчёта — тоже падение, а не «пусть будет ноль».
fn our_containers() -> usize {
    let out = std::process::Command::new("docker")
        .args(["ps", "-q", "--filter", &format!("ancestor={IMAGE}")])
        .output()
        .expect("не выполнить docker ps");
    assert!(out.status.success(), "docker ps завершился с ошибкой");
    String::from_utf8_lossy(&out.stdout).lines().count()
}

#[test]
fn контейнер_удаляется_вместе_с_тестом() {
    assert!(
        docker_available(),
        "Docker не запущен — интеграционные тесты идти не могут"
    );

    let before = our_containers();

    {
        let server = TestServer::start().expect("контейнер не поднялся");
        assert!(
            our_containers() > before,
            "контейнер не появился среди работающих"
        );
        drop(server);
    }

    // Даём Docker миг на уборку.
    std::thread::sleep(std::time::Duration::from_secs(2));

    assert_eq!(
        our_containers(),
        before,
        "после теста остался висящий контейнер"
    );
}
