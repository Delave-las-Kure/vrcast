//! T011 — тест утечки секретов (SC-011, конституция, принцип IV).
//!
//! Проверяется не то, что функция замены работает на строке, а то, что секрет **не находится
//! поиском** после прохождения двумя настоящими путями утечки: через журнал и через сообщение
//! об ошибке. Именно так это и происходит в жизни — не «кто-то напечатал пароль», а
//! «структура с паролем попала в отладочный вывод» или «библиотека вставила его в текст ошибки».

use std::io::Write;
use std::sync::{Arc, Mutex};
use vrcast_studio_lib::store::redact::{self, MASK};
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef, SecretStore};

/// Общий буфер, куда пишет журнал во время теста.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Тесты идут в одном процессе и делят общий список секретов, поэтому не должны
/// пересекаться. Один общий замок — проще и надёжнее, чем изоляция состояния.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn секрет_не_попадает_в_журнал() {
    let _g = lock();
    redact::forget_all();

    // Значение выдумано целиком. Брать образцом настоящий пароль — даже
    // недействующий — нельзя: тест уходит в общедоступный репозиторий вместе
    // со всей историей, и вычистить его оттуда потом заметно дороже.
    let secret = "Zq7#vT2wR9pL4mX-выдуманный-пароль-для-теста";
    redact::register(secret);

    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(redact::RedactingMakeWriter::new(captured.clone()))
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        // Тот самый случай: не «напечатали пароль», а вывели структуру, внутри которой он есть.
        tracing::error!(
            "не удалось подключиться, использованные данные: user=root password={secret}"
        );
        tracing::info!(поле = secret, "поле события тоже проходит через вырезание");
    });

    let out = captured.text();
    assert!(!out.is_empty(), "журнал пуст — тест ничего не проверил");
    assert!(!out.contains(secret), "СЕКРЕТ НАЙДЕН В ЖУРНАЛЕ:\n{out}");
    assert!(out.contains(MASK), "замена не сработала:\n{out}");
}

#[test]
fn секрет_не_попадает_в_сообщение_об_ошибке() {
    let _g = lock();
    redact::forget_all();

    let secret = "парольная-фраза-ключа-9f3a2b";
    redact::register(secret);

    // Ошибка нижележащей библиотеки, которая о наших правилах ничего не знает.
    #[derive(Debug, thiserror::Error)]
    #[error("аутентификация не удалась (передан пароль {0})")]
    struct ChattyError(String);

    let err = ChattyError(secret.to_owned());

    let raw = err.to_string();
    assert!(
        raw.contains(secret),
        "тест построен неверно: секрета нет в исходной ошибке"
    );

    let safe = redact::safe_display(&err);
    assert!(!safe.contains(secret), "СЕКРЕТ НАЙДЕН В ОШИБКЕ: {safe}");
    assert!(safe.contains(MASK));
}

#[test]
fn секрет_из_хранилища_регистрируется_сам() {
    let _g = lock();
    redact::forget_all();

    // Ключевое свойство: вызывающий код НЕ регистрирует секрет вручную.
    // Достаточно того, что секрет прошёл через хранилище.
    let store = InMemorySecretStore::new();
    let reference = SecretRef::for_server("srv_test");
    let secret = "пароль-который-никто-не-регистрировал";

    store.set(&reference, secret).unwrap();

    let leaked = format!("отладка: {secret}");
    assert!(
        !redact::redact(&leaked).contains(secret),
        "секрет, прошедший через хранилище, не был зарегистрирован автоматически"
    );
}

#[test]
fn приватный_ключ_вырезается_даже_без_регистрации() {
    let _g = lock();
    redact::forget_all();

    // Подстраховка для случая, когда ключ прочитан из файла и попал в вывод мимо хранилища —
    // то есть ровно тогда, когда регистрация не сработала.
    let text = "прочитан ключ:\n\
        -----BEGIN OPENSSH PRIVATE KEY-----\n\
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n\
        -----END OPENSSH PRIVATE KEY-----\nконец";

    let out = redact::redact(text);
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA"),
        "тело ключа осталось: {out}"
    );
    assert!(
        !out.contains("BEGIN OPENSSH"),
        "заголовок ключа остался: {out}"
    );
    assert!(
        out.contains("прочитан ключ") && out.contains("конец"),
        "вырезано лишнее: {out}"
    );
}

#[test]
fn обрезанный_ключ_тоже_вырезается_целиком() {
    let _g = lock();
    redact::forget_all();

    // Журнал мог оборваться на середине блока. Лучше потерять хвост, чем показать половину ключа.
    let text = "начало\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk";
    let out = redact::redact(text);
    assert!(
        !out.contains("b3BlbnNzaC1rZXk"),
        "половина ключа осталась: {out}"
    );
    assert!(out.contains("начало"));
}

#[test]
fn многострочный_ключ_не_просачивается_через_построчную_запись() {
    let _g = lock();
    redact::forget_all();

    // Дефект, ради которого тест: журнал пишет событие одним куском с внутренними
    // переводами строк, а писатель резал его построчно и маскировал только строку
    // с «-----BEGIN». Тело ключа — строки чистого base64 — уходило в журнал целым:
    // в отдельной строке нет ни метки блока, ни зарегистрированного секрета.
    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        w.write_all(
            "прочитан ключ:\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n\
             QyNTUxOQAAACDLnp6PPO2Fk3rCC1aXY0P3zXNZDCLYPFHzVv1cnJ2d\n\
             -----END OPENSSH PRIVATE KEY-----\n\
             конец\n"
                .as_bytes(),
        )
        .unwrap();
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA") && !out.contains("QyNTUxOQ"),
        "ТЕЛО КЛЮЧА ПРОШЛО В ЖУРНАЛ: {out}"
    );
    assert!(
        out.contains("прочитан ключ") && out.contains("конец"),
        "вырезано лишнее: {out}"
    );
    assert!(out.contains(MASK), "замена не сработала: {out}");
}

#[test]
fn ключ_пришедший_отдельными_записями_тоже_вырезается() {
    let _g = lock();
    redact::forget_all();

    // Тот же дефект, другой путь: каждая строка ключа приходит отдельным вызовом
    // записи. Писатель обязан копить начатый блок, а не выпускать строки по одной.
    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        for part in [
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
            "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB\n",
            "-----END OPENSSH PRIVATE KEY-----\n",
            "дальше обычный журнал\n",
        ] {
            w.write_all(part.as_bytes()).unwrap();
        }
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(
        !out.contains("b3BlbnNzaC1rZXktdjEA"),
        "ТЕЛО КЛЮЧА ПРОШЛО В ЖУРНАЛ: {out}"
    );
    assert!(
        out.contains("дальше обычный журнал"),
        "вырезано лишнее: {out}"
    );
    assert!(out.contains(MASK));
}

#[test]
fn слишком_короткое_значение_не_регистрируется() {
    let _g = lock();
    redact::forget_all();

    // Секрет из трёх символов встретится в журнале случайно сотни раз и превратит его
    // в решето из масок, скрыв заодно всё полезное. Такая «защита» вредна.
    redact::register("abc");
    let text = "abcdefghij — обычный текст, содержащий abc";
    assert_eq!(
        redact::redact(text),
        text,
        "короткое значение зарегистрировано и испортило текст"
    );
}

#[test]
fn из_двух_вложенных_секретов_вырезается_длинный() {
    let _g = lock();
    redact::forget_all();

    // Если сначала заменить короткий, от длинного останется хвост — а хвост пароля
    // это всё ещё утечка.
    let short = "пароль-часть-1";
    let long = "пароль-часть-1-и-продолжение-2";
    redact::register(short);
    redact::register(long);

    let input = format!("значение: {long}");
    let out = redact::redact(&input);
    assert!(
        !out.contains("продолжение-2"),
        "остался хвост длинного секрета: {out}"
    );
    assert_eq!(out, format!("значение: {MASK}"));
}

#[test]
fn запись_по_частям_не_пропускает_разорванный_секрет() {
    let _g = lock();
    redact::forget_all();

    let secret = "секрет-разорванный-между-записями";
    redact::register(secret);

    let captured = Captured::default();
    {
        let mut w = redact::RedactingWriter::new(captured.clone());
        // Поток вывода может прийти по частям, и секрет окажется разрезан пополам.
        w.write_all("начало секрет-разорван".as_bytes()).unwrap();
        w.write_all("ный-между-записями конец\n".as_bytes())
            .unwrap();
        w.flush().unwrap();
    }

    let out = captured.text();
    assert!(!out.contains(secret), "СЕКРЕТ ПРОШЁЛ ПО ЧАСТЯМ: {out}");
    assert!(out.contains(MASK), "замена не сработала: {out}");
}

#[test]
fn forgetting_one_secret_leaves_the_others_masked() {
    // T073, принцип IV. Удаление одного профиля не должно снимать маскировку
    // с секретов остальных: они живы, и первая же ошибка вынесла бы чужой пароль
    // в журнал — до перезапуска приложения и без единого признака, что что-то не так.
    redact::forget_all();

    let удаляемый = "пароль-удаляемого-профиля-1234";
    let оставшийся = "пароль-другого-профиля-5678";
    redact::register(удаляемый);
    redact::register(оставшийся);

    redact::forget(удаляемый);

    let текст = format!("вход не удался: {удаляемый} и {оставшийся}");
    let вырезано = redact::redact(&текст);

    assert!(
        !вырезано.contains(оставшийся),
        "секрет живого профиля перестал вырезаться: {вырезано}"
    );
    // Забытый больше не вырезается — это и просили, его в хранилище уже нет.
    assert!(вырезано.contains(удаляемый));
}

#[test]
fn deleting_a_profile_unmasks_only_its_own_secret() {
    // Та же беда, но проверенная через настоящее хранилище: важно не то, что
    // `forget` работает, а то, что удаление профиля зовёт именно его.
    use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretRef, SecretStore};

    redact::forget_all();
    let store = InMemorySecretStore::new();

    let первый = SecretRef::for_server("профиль-один");
    let второй = SecretRef::for_server("профиль-два");
    store.set(&первый, "секрет-первого-профиля-1234").unwrap();
    store.set(&второй, "секрет-второго-профиля-5678").unwrap();

    store.delete(&первый).unwrap();

    let вырезано =
        redact::redact("в журнале: секрет-первого-профиля-1234 и секрет-второго-профиля-5678");
    assert!(
        !вырезано.contains("секрет-второго-профиля-5678"),
        "удаление одного профиля сняло маскировку со второго: {вырезано}"
    );
}
