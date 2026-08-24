//! Проверка слоя доступа к серверу на настоящем сервере.
//!
//! Не тест: тесты не должны зависеть от чужого сервера и сети. Это отдельный запуск,
//! которым проверяется, что слой работает против живого OpenSSH, а не только компилируется.
//!
//! Адрес и ключ берутся из окружения — в коде их нет (FR-004):
//!
//! ```text
//! VRCAST_SMOKE_HOST=... VRCAST_SMOKE_USER=root VRCAST_SMOKE_KEY=/путь/к/ключу \
//!   cargo run --example ssh_smoke
//! ```

use std::path::PathBuf;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

#[tokio::main]
async fn main() {
    vrcast_studio_lib::logging::init();

    let host = env("VRCAST_SMOKE_HOST");
    let user = env("VRCAST_SMOKE_USER");
    let key = PathBuf::from(env("VRCAST_SMOKE_KEY"));
    let addr = ServerAddress::new(host, 22);

    println!("== 1. Узнать отпечаток, ничего не предъявляя");
    let fp = fingerprint::probe(&addr)
        .await
        .expect("не удалось узнать отпечаток");
    println!("   отпечаток: {fp}");

    println!("== 2. Подключение с ЗАВЕДОМО ЧУЖИМ отпечатком должно быть отвергнуто");
    let wrong = "SHA256:0000000000000000000000000000000000000000000";
    match Connection::connect(
        addr.clone(),
        &user,
        Credentials::Key {
            path: key.clone(),
            passphrase: None,
        },
        wrong,
    )
    .await
    {
        Err(vrcast_studio_lib::ssh::SshError::HostKeyChanged {
            expected, actual, ..
        }) => {
            println!("   отвергнуто верно: ожидался {expected}, получен {actual}");
        }
        Err(other) => panic!("не та ошибка: {other}"),
        Ok(_) => panic!("ПОДКЛЮЧИЛИСЬ К СЕРВЕРУ С ЧУЖИМ ОТПЕЧАТКОМ — защита не работает"),
    }

    println!("== 3. Подключение с верным отпечатком");
    let conn = Connection::connect(
        addr.clone(),
        &user,
        Credentials::Key {
            path: key.clone(),
            passphrase: None,
        },
        &fp,
    )
    .await
    .expect("подключиться не удалось");
    println!(
        "   вошли как {}, соединение живо: {}",
        conn.user(),
        conn.is_alive()
    );

    println!("== 4. Команда и её код возврата");
    let out = conn
        .exec("hostname; id -un")
        .await
        .expect("команда не выполнилась");
    println!(
        "   код {:?}, вывод: {}",
        out.exit_code,
        out.trimmed().replace('\n', " / ")
    );
    assert!(out.ok(), "команда завершилась неуспешно");

    println!("== 5. Неуспешная команда должна распознаваться как неуспешная");
    let bad = conn.exec("exit 42").await.unwrap();
    assert_eq!(bad.exit_code, Some(42));
    assert!(!bad.ok());
    println!("   код 42 распознан");

    println!("== 6. Поток ошибок отделён от обычного вывода");
    let both = conn.exec("echo НАВЫХОД; echo НАОШИБКИ >&2").await.unwrap();
    assert!(both.stdout.contains("НАВЫХОД"), "потерян обычный вывод");
    assert!(both.stderr.contains("НАОШИБКИ"), "потерян поток ошибок");
    println!("   разделены верно");

    println!("== 7. Несколько каналов одновременно в ОДНОМ соединении");
    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let c = conn.clone();
            tokio::spawn(async move { c.exec(&format!("echo канал-{i}")).await })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        let out = t.await.unwrap().expect("канал не отработал");
        assert!(out.ok());
        ok += 1;
    }
    println!("   {ok} каналов отработали параллельно, соединение одно");

    println!("== 8. Файловые операции");
    let sftp = conn.sftp().await.expect("файловая сессия не открылась");
    let entries = sftp.read_dir("/etc").await.expect("каталог не прочитан");
    println!("   в /etc прочитано записей: {}", entries.count());

    println!("== 9. Состояние сервера, как его увидит распознавание (FR-120)");
    for path in [
        "/etc/vrcast/state.json",
        "/etc/caddy/Caddyfile",
        "/var/lib/vrcast",
    ] {
        let r = conn
            .exec(&format!("test -e {path} && echo ЕСТЬ || echo нет"))
            .await
            .unwrap();
        println!("   {path}: {}", r.trimmed());
    }

    conn.close().await;
    println!("\nВСЕ ПРОВЕРКИ ПРОЙДЕНЫ");
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("не задана переменная окружения {name}"))
}
