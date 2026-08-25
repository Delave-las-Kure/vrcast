//! Ключ для тестов: создаётся на месте, в репозиторий не попадает.
//!
//! Общий для модульных и интеграционных тестов намеренно. Раньше создание жило
//! в оснастке интеграционных, а модульные пользовались готовым файлом — и на
//! машине разработчика это работало, потому что файл там уже лежал от прошлого
//! прогона. В непрерывной интеграции модульные тесты идут отдельным заданием,
//! где интеграционных не было вовсе, — и ключа не оказалось. Поймано первым же
//! прогоном на GitHub 2026-08-25.
//!
//! Почему ключ не хранится в репозитории — см. `tests/fixtures/README.md`.

use std::path::PathBuf;
use std::process::Command;

/// Парольная фраза. Не секрет: ключ создаётся здесь же и не даёт доступа никуда.
pub const PASSPHRASE: &str = "тестовая-фраза-1234";

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Приватный ключ.
pub fn key_path() -> PathBuf {
    fixtures_dir().join("encrypted_ed25519.key")
}

/// Открытая часть — внутри каталога сборки образа контейнера.
pub fn public_key_path() -> PathBuf {
    fixtures_dir().join("docker/encrypted_ed25519.key.pub")
}

/// Есть ли обе половины ключа.
///
/// Именно обе: с одной половиной `ssh-keygen` откажется перезаписывать вторую,
/// а войти по такому ключу всё равно нельзя.
fn готов() -> bool {
    key_path().exists() && public_key_path().exists()
}

/// Создать ключ, если его ещё нет.
///
/// Вызывается из нескольких тестов сразу, и это главная тонкость. Тесты идут
/// в несколько потоков; без замка два из них одновременно решают, что ключа нет,
/// оба стирают половинчатые файлы и оба зовут `ssh-keygen`. Второй натыкается
/// на уже созданный файл, `ssh-keygen` спрашивает про перезапись, читает конец
/// ввода и выходит с ошибкой, не сказав ни слова в поток ошибок. Выглядит это как
/// «ключ для тестов не создался:» с пустотой после двоеточия. Поймано прогоном
/// на Windows 2026-08-25; до того гонка годами не выпадала просто по везению.
pub fn ensure() -> Result<PathBuf, String> {
    static ЗАМОК: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // Отравленный замок не повод отказываться от работы: под ним нет состояния,
    // которое мог бы испортить упавший поток.
    let _держим = ЗАМОК.lock().unwrap_or_else(|e| e.into_inner());

    if готов() {
        return Ok(key_path());
    }

    // Убираем половинчатое состояние: при наличии одного из двух файлов
    // ssh-keygen откажется перезаписывать, и тесты встанут с невнятной ошибкой.
    let generated_pub = fixtures_dir().join("encrypted_ed25519.key.pub");
    let _ = std::fs::remove_file(key_path());
    let _ = std::fs::remove_file(&generated_pub);
    let _ = std::fs::remove_file(public_key_path());

    let out = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-q",
            "-N",
            PASSPHRASE,
            "-C",
            "vrcast-studio: одноразовый ключ для тестов",
            "-f",
        ])
        .arg(key_path())
        .output()
        .map_err(|e| {
            format!(
                "не запустить ssh-keygen: {e}. Он нужен, чтобы создать ключ для тестов; \
                 на Windows входит в состав OpenSSH, на Linux — в openssh-client."
            )
        })?;

    if !out.status.success() {
        // Замок держит только этот процесс. Соседний прогон `cargo` мог создать
        // ключ, пока мы звали `ssh-keygen`, — тогда всё в порядке: нужен готовый
        // ключ, а не наше авторство.
        if готов() {
            return Ok(key_path());
        }
        return Err(format!(
            "ключ для тестов не создался (код {}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    if let Some(parent) = public_key_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::copy(&generated_pub, public_key_path())
        .map_err(|e| format!("открытую часть ключа не положить в каталог сборки: {e}"))?;

    Ok(key_path())
}
