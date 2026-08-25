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

/// Создать ключ, если его ещё нет. Безопасно при одновременном вызове из разных
/// тестов: повторный вызов при готовых файлах ничего не делает.
pub fn ensure() -> Result<PathBuf, String> {
    if key_path().exists() && public_key_path().exists() {
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
        return Err(format!(
            "ключ для тестов не создался:\n{}",
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
