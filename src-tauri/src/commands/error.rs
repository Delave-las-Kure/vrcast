//! T012 — ошибка как объект, а не строка.
//!
//! Договор слоя команд (`contracts/ipc-commands.md`, правило 2): ошибка — это
//! `{ code, message, hint?, cause? }`, где `message` и `hint` уже готовы к показу
//! человеку на русском языке (FR-105).
//!
//! Почему не просто строка. Интерфейсу нужно **различать** причины, чтобы предложить
//! разное действие: «неверный пароль» и «вход по паролю запрещён» выглядят снаружи
//! одинаково, но требуют от пользователя противоположного. Строку для этого пришлось бы
//! разбирать, а разбор строк ломается при первой же правке формулировки.
//!
//! Каждый код обязан иметь и сообщение, и подсказку — это проверяется тестом, а не
//! оставлено на внимательность (конституция, раздел «Ограничения качества исполнения»).

use serde::Serialize;

/// Код ошибки. Перечень закреплён договором `contracts/ipc-commands.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(into = "String")]
pub enum ErrorCode {
    // --- доступ к серверу ---
    SshAuthFailed,
    SshUnreachable,
    HostKeyChanged,
    HostKeyUnconfirmed,
    HostKeyIsCertificate,
    KeyNeedsPassphrase,
    KeyUnreadable,
    VideoDirDenied,

    // --- домен ---
    DomainNotServing,
    DomainNotPointed,
    DomainPointsElsewhere,
    Ipv6Mismatch,

    // --- состояние и развёртывание сервера ---
    ServerForeign,
    ServerTooNew,
    DeployStepFailed,
    SwapFailed,

    // --- библиотека ---
    SlugTaken,
    ManifestConflict,
    FileMissingOnServer,
    FileInUse,

    // --- подготовка файлов ---
    FfmpegBroken,
    NoAudioTracks,
    DecodeValidationFailed,
    NoHwEncoder,
    LocalDiskFull,

    // --- передача ---
    RemoteDiskFull,
    ChecksumMismatch,
    ViewersActive,
    NameExists,

    // --- наборы качеств ---
    RungAboveSource,
    BufsizeTooLarge,
    LevelExceeded,
    LadderIncomplete,
    NoLadderForMedia,

    // --- настройки веб-сервера ---
    CaddyValidateFailed,
    CaddyReloadFailed,

    // --- задачи ---
    TaskCancelled,
    TaskNotFound,
    TaskBadTransition,
    TaskNotPausable,

    // --- прочее ---
    StorageFailed,
    Internal,
}

impl ErrorCode {
    /// Все коды. Существует ради теста полноты: он проходит по этому списку и требует
    /// у каждого кода сообщение и подсказку.
    pub const ALL: &'static [ErrorCode] = &[
        Self::SshAuthFailed,
        Self::SshUnreachable,
        Self::HostKeyChanged,
        Self::HostKeyUnconfirmed,
        Self::HostKeyIsCertificate,
        Self::KeyNeedsPassphrase,
        Self::KeyUnreadable,
        Self::VideoDirDenied,
        Self::DomainNotServing,
        Self::DomainNotPointed,
        Self::DomainPointsElsewhere,
        Self::Ipv6Mismatch,
        Self::ServerForeign,
        Self::ServerTooNew,
        Self::DeployStepFailed,
        Self::SwapFailed,
        Self::SlugTaken,
        Self::ManifestConflict,
        Self::FileMissingOnServer,
        Self::FileInUse,
        Self::FfmpegBroken,
        Self::NoAudioTracks,
        Self::DecodeValidationFailed,
        Self::NoHwEncoder,
        Self::LocalDiskFull,
        Self::RemoteDiskFull,
        Self::ChecksumMismatch,
        Self::ViewersActive,
        Self::NameExists,
        Self::RungAboveSource,
        Self::BufsizeTooLarge,
        Self::LevelExceeded,
        Self::LadderIncomplete,
        Self::NoLadderForMedia,
        Self::CaddyValidateFailed,
        Self::CaddyReloadFailed,
        Self::TaskCancelled,
        Self::TaskNotFound,
        Self::TaskBadTransition,
        Self::TaskNotPausable,
        Self::StorageFailed,
        Self::Internal,
    ];

    /// Строковый код, уходящий в интерфейс.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SshAuthFailed => "SSH_AUTH_FAILED",
            Self::SshUnreachable => "SSH_UNREACHABLE",
            Self::HostKeyChanged => "HOST_KEY_CHANGED",
            Self::HostKeyUnconfirmed => "HOST_KEY_UNCONFIRMED",
            Self::HostKeyIsCertificate => "HOST_KEY_IS_CERTIFICATE",
            Self::KeyNeedsPassphrase => "KEY_NEEDS_PASSPHRASE",
            Self::KeyUnreadable => "KEY_UNREADABLE",
            Self::VideoDirDenied => "VIDEO_DIR_DENIED",
            Self::DomainNotServing => "DOMAIN_NOT_SERVING",
            Self::DomainNotPointed => "DOMAIN_NOT_POINTED",
            Self::DomainPointsElsewhere => "DOMAIN_POINTS_ELSEWHERE",
            Self::Ipv6Mismatch => "IPV6_MISMATCH",
            Self::ServerForeign => "SERVER_FOREIGN",
            Self::ServerTooNew => "SERVER_TOO_NEW",
            Self::DeployStepFailed => "DEPLOY_STEP_FAILED",
            Self::SwapFailed => "SWAP_FAILED",
            Self::SlugTaken => "SLUG_TAKEN",
            Self::ManifestConflict => "MANIFEST_CONFLICT",
            Self::FileMissingOnServer => "FILE_MISSING_ON_SERVER",
            Self::FileInUse => "FILE_IN_USE",
            Self::FfmpegBroken => "FFMPEG_BROKEN",
            Self::NoAudioTracks => "NO_AUDIO_TRACKS",
            Self::DecodeValidationFailed => "DECODE_VALIDATION_FAILED",
            Self::NoHwEncoder => "NO_HW_ENCODER",
            Self::LocalDiskFull => "LOCAL_DISK_FULL",
            Self::RemoteDiskFull => "REMOTE_DISK_FULL",
            Self::ChecksumMismatch => "CHECKSUM_MISMATCH",
            Self::ViewersActive => "VIEWERS_ACTIVE",
            Self::NameExists => "NAME_EXISTS",
            Self::RungAboveSource => "RUNG_ABOVE_SOURCE",
            Self::BufsizeTooLarge => "BUFSIZE_TOO_LARGE",
            Self::LevelExceeded => "LEVEL_EXCEEDED",
            Self::LadderIncomplete => "LADDER_INCOMPLETE",
            Self::NoLadderForMedia => "NO_LADDER_FOR_MEDIA",
            Self::CaddyValidateFailed => "CADDY_VALIDATE_FAILED",
            Self::CaddyReloadFailed => "CADDY_RELOAD_FAILED",
            Self::TaskCancelled => "TASK_CANCELLED",
            Self::TaskNotFound => "TASK_NOT_FOUND",
            Self::TaskBadTransition => "TASK_BAD_TRANSITION",
            Self::TaskNotPausable => "TASK_NOT_PAUSABLE",
            Self::StorageFailed => "STORAGE_FAILED",
            Self::Internal => "INTERNAL",
        }
    }

    /// Что произошло — словами, понятными человеку.
    pub fn message(&self) -> &'static str {
        match self {
            Self::SshAuthFailed => "Сервер не принял данные для входа",
            Self::SshUnreachable => "Не удалось связаться с сервером",
            Self::HostKeyChanged => "Отпечаток сервера изменился",
            Self::HostKeyUnconfirmed => "Отпечаток сервера ещё не подтверждён",
            Self::HostKeyIsCertificate => "Сервер предъявил сертификат вместо ключа",
            Self::KeyNeedsPassphrase => "Ключ защищён парольной фразой",
            Self::KeyUnreadable => "Файл ключа не удалось прочитать",
            Self::VideoDirDenied => "Нет доступа к каталогу с видео на сервере",
            Self::DomainNotServing => "Домен не отдаёт видео",
            Self::DomainNotPointed => "Домен не прикреплён к серверу",
            Self::DomainPointsElsewhere => "Домен ведёт на другой сервер",
            Self::Ipv6Mismatch => "Выбор по IPv6 не сходится с доменными записями",
            Self::ServerForeign => "На сервере уже настроена посторонняя раздача",
            Self::ServerTooNew => "Серверная часть новее, чем понимает это приложение",
            Self::DeployStepFailed => "Шаг развёртывания не выполнился",
            Self::SwapFailed => "Не удалось создать файл подкачки",
            Self::SlugTaken => "Такое имя уже занято",
            Self::ManifestConflict => "Опись изменена другим приложением",
            Self::FileMissingOnServer => "Файла больше нет на сервере",
            Self::FileInUse => "Этот файл сейчас смотрят",
            Self::FfmpegBroken => "Средство обработки видео не запускается",
            Self::NoAudioTracks => "В файле нет ни одной звуковой дорожки",
            Self::DecodeValidationFailed => "Готовый файл не прошёл проверку воспроизведения",
            Self::NoHwEncoder => "Аппаратное ускорение недоступно",
            Self::LocalDiskFull => "На диске компьютера не хватает места",
            Self::RemoteDiskFull => "На диске сервера не хватает места",
            Self::ChecksumMismatch => "Переданный файл отличается от исходного",
            Self::ViewersActive => "Прямо сейчас идёт просмотр",
            Self::NameExists => "Файл с таким именем уже раздаётся",
            Self::RungAboveSource => "Ступень качества выше, чем сам источник",
            Self::BufsizeTooLarge => "Размер буфера слишком велик для заданного предела пиков",
            Self::LevelExceeded => "Поток не укладывается в выбранный уровень совместимости",
            Self::LadderIncomplete => "Набор качеств собран не полностью",
            Self::NoLadderForMedia => "У этого медиа нет набора качеств",
            Self::CaddyValidateFailed => "Новые настройки сервера оказались неверными",
            Self::CaddyReloadFailed => "Сервер не принял новые настройки",
            Self::TaskCancelled => "Задача отменена",
            Self::TaskNotFound => "Задача не найдена",
            Self::TaskBadTransition => "Задача сейчас в состоянии, из которого это невозможно",
            Self::TaskNotPausable => "Задачу этого вида нельзя приостановить",
            Self::StorageFailed => "Не удалось обратиться к локальному хранилищу",
            Self::Internal => "Внутренняя ошибка приложения",
        }
    }

    /// Что делать. Это не украшение: сообщение без подсказки оставляет человека
    /// наедине с проблемой, о которой он ничего не знает.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::SshAuthFailed => {
                "Проверьте имя пользователя и пароль или ключ. Если сервер предлагает только вход по ключу, пароль не подойдёт — заведите ключ."
            }
            Self::SshUnreachable => {
                "Проверьте, что сервер включён, адрес указан верно и порт доступен снаружи."
            }
            Self::HostKeyChanged => {
                "Так бывает после пересоздания сервера — тогда подтвердите новый отпечаток. Если сервер не менялся, не подключайтесь: возможна подмена."
            }
            Self::HostKeyUnconfirmed => {
                "Сравните показанный отпечаток с тем, что даёт хостер, и подтвердите его."
            }
            Self::HostKeyIsCertificate => {
                "Приложение работает с серверами, предъявляющими обычный ключ. Отключите на сервере выдачу сертификата хоста."
            }
            Self::KeyNeedsPassphrase => "Введите парольную фразу этого ключа.",
            Self::KeyUnreadable => {
                "Проверьте путь к файлу и что это именно приватный ключ, а не открытая его часть (с расширением .pub)."
            }
            Self::VideoDirDenied => {
                "Убедитесь, что путь указан верно и у пользователя есть права на этот каталог."
            }
            Self::DomainNotServing => {
                "Раздача не отвечает по домену. Проверьте состояние сервера в разделе диагностики."
            }
            Self::DomainNotPointed => {
                "Создайте у своего регистратора запись типа A для этого имени, указав адрес сервера. Изменение расходится по сети несколько минут."
            }
            Self::DomainPointsElsewhere => {
                "Исправьте запись A так, чтобы она указывала на адрес этого сервера."
            }
            Self::Ipv6Mismatch => {
                "Либо заведите запись AAAA на адрес IPv6 этого сервера, либо выберите отключение IPv6 при развёртывании."
            }
            Self::ServerForeign => {
                "Приложение не трогает чужие настройки. Возьмите чистый сервер или уберите постороннюю раздачу вручную."
            }
            Self::ServerTooNew => {
                "Обновите приложение: работать с сервером, устройства которого оно не понимает, небезопасно."
            }
            Self::DeployStepFailed => {
                "Посмотрите, на каком шаге остановилось, и запустите развёртывание снова — выполненные шаги повторяться не будут."
            }
            Self::SwapFailed => {
                "Освободите место на диске сервера: файлу подкачки нужен как минимум гигабайт."
            }
            Self::SlugTaken => "Выберите другое имя: это уже занято другим медиа.",
            Self::ManifestConflict => {
                "С этим сервером работает ещё одно приложение. Обновите список и повторите действие."
            }
            Self::FileMissingOnServer => {
                "Файл удалили мимо приложения. Обновите библиотеку, чтобы список пришёл в соответствие."
            }
            Self::FileInUse => {
                "Удаление или переименование оборвёт просмотр. Дождитесь окончания или подтвердите действие осознанно."
            }
            Self::FfmpegBroken => {
                "Переустановите приложение: входящее в него средство обработки видео повреждено."
            }
            Self::NoAudioTracks => {
                "Выберите другой исходник: без звука раздавать нечего."
            }
            Self::DecodeValidationFailed => {
                "Файл повреждён и к раздаче не годится. Попробуйте подготовить его заново из исходника."
            }
            Self::NoHwEncoder => {
                "Подготовка пойдёт на процессоре — дольше, но так же качественно. Если ускорение должно быть, закройте программы, занявшие видеокарту."
            }
            Self::LocalDiskFull => "Освободите место на диске компьютера и повторите.",
            Self::RemoteDiskFull => {
                "Освободите место на сервере: удалите ненужные медиа в библиотеке."
            }
            Self::ChecksumMismatch => {
                "Передача повреждена. Файл в раздачу не попал — запустите заливку снова."
            }
            Self::ViewersActive => {
                "Заливка вымоет из памяти сервера то, что сейчас смотрят, и у зрителей пойдут подвисания. Лучше дождаться конца просмотра."
            }
            Self::NameExists => {
                "Выберите другое имя или подтвердите замену. Помните, что кешированная копия у CDN какое-то время будет отдавать старое."
            }
            Self::RungAboveSource => {
                "Понизьте ступень: детали, которой нет в источнике, не появится, а файл раздуется."
            }
            Self::BufsizeTooLarge => {
                "Сделайте буфер примерно равным пределу пиков, иначе реальные пики превысят предел и у зрителей пойдут замирания."
            }
            Self::LevelExceeded => {
                "Уровень считается по двум пределам — на кадр и в секунду. Понизьте битрейт, частоту кадров или разрешение."
            }
            Self::LadderIncomplete => {
                "Часть вариантов не отдаётся. Запустите сборку снова — готовые пересобираться не будут."
            }
            Self::NoLadderForMedia => {
                "Сначала соберите набор качеств: ограничивать качество можно только выбором из существующих ступеней."
            }
            Self::CaddyValidateFailed => {
                "Изменение не применено, раздача работает по-прежнему. Сообщите об этой ошибке — это неполадка приложения."
            }
            Self::CaddyReloadFailed => {
                "Прежние настройки возвращены, раздача работает. Проверьте состояние сервера в разделе диагностики."
            }
            Self::TaskCancelled => "Ничего делать не нужно: задачу сняли по вашей команде.",
            Self::TaskNotFound => {
                "Задача уже завершилась или была снята. Обновите список задач."
            }
            Self::TaskBadTransition => "Обновите список задач: их состояние изменилось.",
            Self::TaskNotPausable => {
                "Короткие задачи не приостанавливают — их проще отменить и выполнить заново."
            }
            Self::StorageFailed => {
                "Проверьте, что на диске есть место и у приложения есть права на свой каталог данных."
            }
            Self::Internal => {
                "Сообщите об этой ошибке. Если она повторяется, помогут журналы из раздела диагностики."
            }
        }
    }
}

impl From<ErrorCode> for String {
    fn from(c: ErrorCode) -> Self {
        c.as_str().to_owned()
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ошибка, уходящая в интерфейс.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub hint: String,
    /// Подробность, уточняющая случай: какой файл, какой шаг, какой адрес.
    /// Всегда проходит вырезание секретов.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.message().to_owned(),
            hint: code.hint().to_owned(),
            cause: None,
        }
    }

    /// Добавить подробность. Проходит вырезание секретов: подробность нередко приходит
    /// от чужой библиотеки, которая о наших правилах не знает (конституция, принцип IV).
    pub fn with_cause(mut self, cause: impl std::fmt::Display) -> Self {
        self.cause = Some(crate::store::redact::safe_display(&cause));
        self
    }

    /// Заменить сообщение на уточнённое — когда общей формулировки мало.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = crate::store::redact::redact(&message.into()).into_owned();
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;

// --- перевод ошибок нижних слоёв в коды договора ---

impl From<crate::ssh::SshError> for AppError {
    fn from(e: crate::ssh::SshError) -> Self {
        use crate::ssh::SshError as S;
        let code = match &e {
            S::Unreachable { .. } => ErrorCode::SshUnreachable,
            S::HostKeyChanged { .. } => ErrorCode::HostKeyChanged,
            S::HostKeyUnconfirmed { .. } => ErrorCode::HostKeyUnconfirmed,
            S::HostKeyIsCertificate => ErrorCode::HostKeyIsCertificate,
            S::AuthFailed { .. } => ErrorCode::SshAuthFailed,
            S::KeyNeedsPassphrase { .. } => ErrorCode::KeyNeedsPassphrase,
            S::KeyUnreadable { .. } => ErrorCode::KeyUnreadable,
            S::Exec(_) | S::Protocol(_) => ErrorCode::Internal,
            S::Sftp(_) => ErrorCode::VideoDirDenied,
        };
        // Подробность нижнего слоя сохраняем: она называет конкретику — какой адрес,
        // какие способы входа предложил сервер, какой файл ключа.
        AppError::new(code).with_cause(e)
    }
}

impl From<crate::store::db::DbError> for AppError {
    fn from(e: crate::store::db::DbError) -> Self {
        AppError::new(ErrorCode::StorageFailed).with_cause(e)
    }
}

impl From<crate::store::secrets::SecretError> for AppError {
    fn from(e: crate::store::secrets::SecretError) -> Self {
        AppError::new(ErrorCode::StorageFailed).with_cause(e)
    }
}

impl From<crate::tasks::engine::TaskError> for AppError {
    fn from(e: crate::tasks::engine::TaskError) -> Self {
        use crate::tasks::engine::TaskError as T;
        let code = match &e {
            T::NotFound(_) => ErrorCode::TaskNotFound,
            T::BadTransition { .. } => ErrorCode::TaskBadTransition,
            T::NotPausable => ErrorCode::TaskNotPausable,
            T::Cancelled => ErrorCode::TaskCancelled,
            T::Db(_) => ErrorCode::StorageFailed,
            T::Failed(_) => ErrorCode::Internal,
        };
        AppError::new(code).with_cause(e)
    }
}
