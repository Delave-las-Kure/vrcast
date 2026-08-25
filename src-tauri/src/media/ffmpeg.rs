//! T115 — вложенный в поставку FFmpeg: где он лежит и что умеет (FR-112, R-01).
//!
//! Приложение ничего не скачивает при первом запуске. Человек ставит его и сразу
//! готовит видео; сборка FFmpeg кладётся рядом при сборке установщика
//! (`scripts/fetch-ffmpeg.mjs`, слепок закреплён в `scripts/ffmpeg.json`).
//!
//! **Проверять при запуске обязательно.** Вложенный файл может не запуститься:
//! его вырезал антивирус, установщик распаковался наполовину, у файла нет права
//! на выполнение. Узнать об этом в начале — значит сказать человеку, что чинить.
//! Узнать в середине двухчасовой подготовки — значит отнять эти два часа.
//!
//! Здесь нет разбора исходников и запуска подготовки: это `probe.rs` и `convert.rs`.
//! Здесь — только «есть ли чем работать вообще».

use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error("вложенный FFmpeg не найден: искали {0}")]
    NotFound(String),

    #[error("вложенный FFmpeg не запускается: {0}")]
    NotRunnable(String),

    #[error("вложенный FFmpeg отвечает не тем, чем должен: {0}")]
    Unexpected(String),
}

pub type Result<T> = std::result::Result<T, FfmpegError>;

/// Что удалось узнать о вложенной сборке.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FfmpegInfo {
    /// Строка версии как её называет сама программа.
    pub version: String,
    /// Полный путь — нужен при разборе неполадок.
    pub path: String,
    /// Есть ли программный кодировщик H.264. Без него подготовка невозможна вовсе.
    pub has_x264: bool,
    /// Аппаратные кодировщики H.264, которые сборка умеет звать.
    ///
    /// «Умеет звать» — не «работают здесь»: наличие в сборке ничего не говорит
    /// о видеокарте. Настоящая проверка — пробный запуск, и она отдельно (FR-026).
    pub hardware: Vec<String>,
}

/// Аппаратные кодировщики H.264, которые нас интересуют.
///
/// Порядок — предпочтения: NVIDIA быстрее прочих на нашем материале, дальше AMD
/// и встроенное в процессор Intel.
const HW_ENCODERS: [&str; 3] = ["h264_nvenc", "h264_amf", "h264_qsv"];

/// Имя вложенной программы рядом с приложением.
///
/// Сборщик кладёт вложенные программы рядом с исполняемым файлом, отрезав от имени
/// тройку платформы: `ffmpeg-x86_64-pc-windows-msvc.exe` становится `ffmpeg.exe`.
fn bundled_name(tool: &str) -> String {
    format!("{tool}{}", std::env::consts::EXE_SUFFIX)
}

/// Найти вложенную программу.
///
/// Сначала рядом с приложением — там она лежит у установленного приложения.
/// В отладочной сборке добавляется каталог, куда её кладёт `npm run ffmpeg`:
/// иначе разработка и тесты требовали бы каждый раз собирать установщик.
/// В собранном приложении этого запасного пути нет намеренно — он указывал бы
/// на каталог чужой машины.
pub fn locate(tool: &str) -> Result<PathBuf> {
    let name = bundled_name(tool);
    let mut tried: Vec<String> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
            tried.push(candidate.display().to_string());
        }
    }

    #[cfg(debug_assertions)]
    {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!(
                "{tool}-{}{}",
                target_triple(),
                std::env::consts::EXE_SUFFIX
            ));
        if dev.is_file() {
            return Ok(dev);
        }
        tried.push(dev.display().to_string());
    }

    Err(FfmpegError::NotFound(tried.join(", ")))
}

/// Тройка платформы, под которую собрано приложение.
///
/// Собирается из тех же составляющих, что использует сборщик: отдельного способа
/// спросить её у самой программы нет.
#[cfg(debug_assertions)]
fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "windows" => format!("{arch}-pc-windows-msvc"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "macos" => format!("{arch}-apple-darwin"),
        other => format!("{arch}-{other}"),
    }
}

/// Проверить вложенную сборку: запускается ли и умеет ли то, что нужно.
pub async fn probe_self() -> Result<FfmpegInfo> {
    let path = locate("ffmpeg")?;
    // ffprobe нужен наравне с ffmpeg: без него не разобрать исходник. Отсутствие
    // одного из двух — та же беда, и сказать о ней надо здесь, а не при первом разборе.
    locate("ffprobe")?;

    let version = parse_version(&run(&path, &["-hide_banner", "-version"]).await?)?;
    let encoders = run(&path, &["-hide_banner", "-encoders"]).await?;

    Ok(FfmpegInfo {
        version,
        path: path.display().to_string(),
        has_x264: encoder_present(&encoders, "libx264"),
        hardware: HW_ENCODERS
            .iter()
            .filter(|n| encoder_present(&encoders, n))
            .map(|n| (*n).to_owned())
            .collect(),
    })
}

/// Вытащить версию из ответа программы.
///
/// Отдельно от запуска, чтобы разбор проверялся тестом без вложенного файла:
/// в непрерывной интеграции его нет — сто сорок мегабайт качать на каждый прогон
/// незачем, — и без этого разделения разбор остался бы непроверенным вовсе.
pub fn parse_version(text: &str) -> Result<String> {
    let line = first_line(text)
        .ok_or_else(|| FfmpegError::Unexpected(String::from("пустой ответ на запрос версии")))?;

    // Проверка не придирка: под именем `ffmpeg` в системе может оказаться что угодно —
    // от обёртки пакетного менеджера до сообщения «программа не установлена».
    if !line.starts_with("ffmpeg version") {
        return Err(FfmpegError::Unexpected(format!(
            "на запрос версии ответил «{line}»"
        )));
    }
    Ok(line)
}

/// Есть ли такой кодировщик в перечне.
///
/// Смотрится **только столбец имён**, а не весь перечень. Поиск по всему тексту
/// не годится, и это не теоретическая придирка: в строке
/// `V....D h264_nvenc  NVIDIA NVENC H.264 encoder` слово «NVENC» стоит ещё и в
/// человеческом описании, так что поиск по словам объявил бы наличие кодировщика
/// `nvenc`, которого не существует. Приложение решило бы, что аппаратное ускорение
/// доступно, и упало бы уже на запуске подготовки.
///
/// Поиск подстрокой тем более не годится: `x264` нашлось бы внутри `libx264`,
/// а `aac` — внутри доброго десятка чужих имён.
pub fn encoder_present(listing: &str, name: &str) -> bool {
    encoder_names(listing).any(|n| n.eq_ignore_ascii_case(name))
}

/// Имена кодировщиков из перечня.
///
/// Строка перечня выглядит так: ` V....D libx264   libx264 H.264 / AVC`. Первый
/// столбец — свойства (вид потока и что кодировщик умеет), второй — имя, дальше
/// описание для человека. Опознаём строку по первому столбцу: он ровно шесть знаков
/// и состоит из известного набора. Заголовок и пояснения так отсеиваются сами,
/// без привязки к их виду, — а он от версии к версии меняется.
fn encoder_names(listing: &str) -> impl Iterator<Item = &str> {
    /// Знаки, из которых состоит столбец свойств.
    const FLAGS: &str = "VASFXBDIL.";

    listing.lines().filter_map(|line| {
        let mut parts = line.split_whitespace();
        let flags = parts.next()?;
        if flags.len() != 6 || !flags.chars().all(|c| FLAGS.contains(c)) {
            return None;
        }
        parts.next()
    })
}

async fn run(path: &Path, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| FfmpegError::NotRunnable(format!("{}: {e}", path.display())))?;

    // `-encoders` и `-version` пишут в обычный вывод, но при части сборок часть
    // сведений уходит в поток ошибок. Берём оба: разделять их здесь незачем.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    if !out.status.success() && text.trim().is_empty() {
        return Err(FfmpegError::NotRunnable(format!(
            "{} завершился с {} и ничего не сказал",
            path.display(),
            out.status
        )));
    }
    Ok(text)
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}
