//! Проверка слоя доступа к серверу против настоящего OpenSSH.
//!
//! Здесь проверяется то, что нельзя проверить без сервера: вход по ключу с парольной
//! фразой доходит до конца, отпечаток совпадает с тем, что считает сам сервер, отказ
//! при чужом отпечатке происходит **до** отправки учётных данных, каналы
//! мультиплексируются в одном соединении, файловые операции работают.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE, ROOT_PASSWORD};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress, SshError};

fn addr(server: &TestServer) -> ServerAddress {
    ServerAddress::new(server.host(), server.port)
}

fn key_credentials() -> Credentials {
    Credentials::Key {
        path: key_path(),
        passphrase: Some(KEY_PASSPHRASE.to_owned()),
    }
}

async fn connect(server: &TestServer) -> Connection {
    let a = addr(server);
    let fp = fingerprint::probe(&a).await.expect("отпечаток не получен");
    Connection::connect(a, "root", key_credentials(), &fp)
        .await
        .expect("подключиться не удалось")
}

#[tokio::test]
async fn отпечаток_совпадает_с_тем_что_считает_сам_сервер() {
    let server = TestServer::start().expect("контейнер не поднялся");

    let ours = fingerprint::probe(&addr(&server))
        .await
        .expect("отпечаток не получен");

    // Сверяем не с самими собой, а со средством сервера: иначе проверили бы только то,
    // что наш код устойчиво выдаёт одно и то же значение — верное оно или нет.
    let theirs = server
        .exec_inside("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | awk '{print $2}'")
        .expect("сервер не назвал свой отпечаток");

    assert_eq!(
        ours.trim(),
        theirs.trim(),
        "наш отпечаток расходится с тем, что считает сервер"
    );
}

#[tokio::test]
async fn вход_по_ключу_с_парольной_фразой_доходит_до_конца() {
    // FR-096. Модульный тест проверяет только разбор ключа; здесь — что по нему
    // действительно пускают на настоящий OpenSSH.
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;

    let out = conn.exec("id -un").await.expect("команда не выполнилась");
    assert!(out.ok(), "команда завершилась неуспешно");
    assert_eq!(out.trimmed(), "root");
}

#[tokio::test]
async fn ключ_без_парольной_фразы_не_проходит() {
    let server = TestServer::start().expect("контейнер не поднялся");
    let a = addr(&server);
    let fp = fingerprint::probe(&a).await.unwrap();

    let err = Connection::connect(
        a,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: None,
        },
        &fp,
    )
    .await
    .expect_err("вошли защищённым ключом без парольной фразы");

    assert!(
        matches!(err, SshError::KeyNeedsPassphrase { .. }),
        "получена не та ошибка: {err}"
    );
}

#[tokio::test]
async fn вход_по_паролю_работает_и_неверный_пароль_отвергается() {
    let server = TestServer::start().expect("контейнер не поднялся");
    let a = addr(&server);
    let fp = fingerprint::probe(&a).await.unwrap();

    let conn = Connection::connect(
        a.clone(),
        "root",
        Credentials::Password(ROOT_PASSWORD.to_owned()),
        &fp,
    )
    .await
    .expect("вход по верному паролю не удался");
    assert!(conn.exec("true").await.unwrap().ok());
    conn.close().await;

    let err = Connection::connect(
        a,
        "root",
        Credentials::Password(String::from("совершенно-не-тот-пароль")),
        &fp,
    )
    .await
    .expect_err("вошли с неверным паролем");

    match err {
        SshError::AuthFailed { methods } => {
            // Перечень предложенных способов — то, что отличает «неверный пароль»
            // от «вход по паролю запрещён». Он обязан быть непустым.
            assert!(
                !methods.is_empty(),
                "сервер не назвал ни одного способа входа"
            );
        }
        other => panic!("получена не та ошибка: {other}"),
    }

    // Неверный пароль ДОШЁЛ до сервера — sshd записал отвергнутую попытку. Эта же
    // строка ищется тестом про чужой отпечаток, только там её быть НЕ должно:
    // здесь доказывается, что метка настоящая и появляется, когда данные ушли.
    server
        .wait_in_sshd_log("Failed password", std::time::Duration::from_secs(10))
        .expect("сервер не записал отвергнутую попытку входа");
}

#[tokio::test]
async fn при_чужом_отпечатке_учётные_данные_не_отправляются() {
    // Решение строже спецификации (см. ssh/fingerprint.rs): отказ происходит на уровне
    // рукопожатия. Проверяем это не по нашей ошибке, а по журналу самого сервера —
    // в нём не должно быть ни одной попытки входа.
    let server = TestServer::start().expect("контейнер не поднялся");

    let err = Connection::connect(
        addr(&server),
        "root",
        Credentials::Password(String::from("пароль-который-не-должен-уйти-на-сервер")),
        "SHA256:заведомоЧужойОтпечатокСервера0000000000000",
    )
    .await
    .expect_err("подключились к серверу с чужим отпечатком");

    assert!(
        matches!(err, SshError::HostKeyChanged { .. }),
        "получена не та ошибка: {err}"
    );

    // Сверяемся с журналом sshd. Сначала — что журнал вообще видит наше подключение:
    // оборванное на рукопожатии, оно оставляет след «[preauth]». Без этой проверки
    // утверждение ниже было бы пустым — тишина в журнале «доказывала» бы что угодно.
    let log = server
        .wait_in_sshd_log("[preauth]", std::time::Duration::from_secs(10))
        .expect("в журнале sshd нет и следа нашего подключения — сверка смотрит не туда");

    // Главное: ни одной попытки входа. «Failed password» — настоящая метка sshd,
    // доказано соседним тестом, где она обязана появляться.
    assert!(
        !log.contains("Failed password") && !log.contains("Accepted password"),
        "на сервер ушла попытка входа, хотя отпечаток не совпал. Журнал:\n{log}"
    );
}

#[tokio::test]
async fn много_каналов_в_одном_соединении() {
    // R-04: сервер ограничивает число одновременно устанавливаемых соединений, и именно
    // на этом однажды оборвалась сборка лесенки. Каналы должны идти внутри одного.
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;

    // Двенадцать — намеренно больше предела сервера (MaxSessions 10). Слой обязан
    // выстроить их в очередь, а не отказать: превышение предела не ошибка пользователя.
    let mut handles = Vec::new();
    for i in 0..12 {
        let c = conn.clone();
        handles.push(tokio::spawn(async move {
            c.exec(&format!("echo канал-{i}")).await
        }));
    }

    for h in handles {
        let out = h.await.unwrap().expect("канал не отработал");
        assert!(out.ok(), "команда в канале завершилась неуспешно");
    }

    // Сервер должен видеть одно соединение, а не двенадцать. Сбой самой сверки —
    // падение, а не запасное значение: с «unwrap_or(1)» проверка проходила бы и
    // при отсутствующем ss (его не было в образе), то есть не проверяла ничего.
    let established = server
        .exec_inside("ss -tn state established '( sport = :22 )' | tail -n +2 | wc -l")
        .expect("не сосчитать соединения средствами сервера");
    let count: usize = established
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("вывод ss не разобрать: «{established}»"));
    assert!(
        (1..=2).contains(&count),
        "сервер видит {count} соединений вместо одного — мультиплексирование не работает"
    );
}

#[tokio::test]
async fn поток_ошибок_отделён_от_обычного_вывода() {
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;

    let out = conn
        .exec("echo НАВЫХОД; echo НАОШИБКИ >&2; exit 3")
        .await
        .expect("команда не выполнилась");

    assert_eq!(out.exit_code, Some(3), "код возврата потерян");
    assert!(!out.ok());
    assert!(out.stdout.contains("НАВЫХОД"), "потерян обычный вывод");
    assert!(out.stderr.contains("НАОШИБКИ"), "потерян поток ошибок");
}

#[tokio::test]
async fn файловые_операции_работают() {
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;
    let sftp = conn.sftp().await.expect("файловая сессия не открылась");

    // Каталог раздачи в контейнере там же, где на настоящем сервере.
    let entries = sftp
        .read_dir("/var/lib/vrcast")
        .await
        .expect("каталог не прочитан");
    let names: Vec<String> = entries.map(|e| e.file_name()).collect();
    assert!(
        names.iter().any(|n| n == "videos"),
        "каталога videos нет: {names:?}"
    );

    // Запись и чтение обратно — то, на что обопрётся передача с возобновлением.
    //
    // ВНИМАНИЕ на выбор вызова: у библиотеки `write` открывает файл ТОЛЬКО на запись,
    // без создания, и на несуществующем пути даёт «нет такого файла». Создаёт `create`.
    // Имя обещает одно, поведение другое — на этом легко напороться при заливке.
    let path = "/var/lib/vrcast/videos/проверка.txt";
    {
        use tokio::io::AsyncWriteExt;
        let mut file = sftp.create(path).await.expect("файл не создался");
        file.write_all("содержимое проверки".as_bytes())
            .await
            .expect("файл не записался");
        file.flush().await.expect("файл не дописался");
    }

    let read_back = server
        .exec_inside(&format!("cat '{path}'"))
        .expect("файл не прочитался средствами сервера");
    assert_eq!(read_back.trim(), "содержимое проверки");

    let meta = sftp.metadata(path).await.expect("сведений о файле нет");
    assert_eq!(meta.size, Some("содержимое проверки".len() as u64));

    sftp.remove_file(path).await.expect("файл не удалился");
    assert!(
        server.exec_inside(&format!("test -e '{path}'")).is_err(),
        "файл остался после удаления"
    );
}

#[tokio::test]
async fn обрыв_соединения_замечается() {
    // Обрывы — норма работы, а не исключение. Приложение обязано их замечать,
    // а не считать соединение живым вечно.
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;
    assert!(conn.is_alive(), "свежее соединение считается мёртвым");

    // Роняем сервер под соединением.
    let _ = server.exec_inside("pkill -f 'sshd: root' || true");
    drop(server);

    // Команда обязана завершиться ошибкой — быстро и явно. Раньше ветка Err(_)
    // (истечение 20 секунд, то есть ЗАВИСАНИЕ) засчитывалась как успех, и тест
    // не мог упасть ни при какой реализации.
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(20), conn.exec("echo жив")).await;

    match result {
        Ok(Err(_)) => {}
        Ok(Ok(out)) => panic!("команда выполнилась на мёртвом соединении: {out:?}"),
        Err(_) => panic!("обрыв не замечен: команда провисела 20 секунд вместо ошибки"),
    }
}
