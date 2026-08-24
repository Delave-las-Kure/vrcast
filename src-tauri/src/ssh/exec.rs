//! T025 — выполнение команд на сервере.

use super::{Connection, Result, SshError};
use russh::ChannelMsg;

/// Результат выполнения команды.
///
/// Код возврата — `Option`: сервер не обязан его прислать, если канал оборвался.
/// Отсутствие кода это НЕ успех, и вызывающий код обязан различать эти случаи.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// Успех — только явный нулевой код возврата.
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Вывод без хвостовых переводов строки — то, что обычно нужно от однострочной команды.
    pub fn trimmed(&self) -> &str {
        self.stdout.trim_end()
    }

    /// Превратить неуспех в ошибку с понятным текстом.
    pub fn require_ok(self, what: &str) -> Result<Self> {
        if self.ok() {
            return Ok(self);
        }
        let code = match self.exit_code {
            Some(c) => c.to_string(),
            None => String::from("нет кода возврата, канал оборвался"),
        };
        let detail = if self.stderr.trim().is_empty() {
            self.stdout.trim().to_owned()
        } else {
            self.stderr.trim().to_owned()
        };
        Err(SshError::Exec(format!("{what}: код {code}. {detail}")))
    }
}

impl Connection {
    /// Выполнить команду и дождаться её завершения.
    ///
    /// Открывает отдельный канал в уже установленном соединении — новое соединение
    /// не создаётся (R-04: сервер ограничивает число одновременно устанавливаемых).
    pub async fn exec(&self, command: &str) -> Result<CommandOutput> {
        let mut channel = self
            .handle()
            .channel_open_session()
            .await
            .map_err(SshError::protocol)?;

        channel
            .exec(true, command)
            .await
            .map_err(SshError::protocol)?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = None;

        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
                // Поток ошибок приходит отдельным типом сообщения; ext == 1 это stderr.
                ChannelMsg::ExtendedData { ref data, ext: 1 } => stderr.extend_from_slice(data),
                ChannelMsg::ExitStatus { exit_status } => {
                    // Выходить из цикла сразу нельзя: за кодом возврата может прийти
                    // ещё не считанный вывод.
                    exit_code = Some(exit_status);
                }
                _ => {}
            }
        }

        Ok(CommandOutput {
            exit_code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}
