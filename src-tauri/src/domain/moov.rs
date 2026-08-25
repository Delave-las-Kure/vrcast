//! T033a — разбор атома `moov`: параметры видео прямо из заголовка MP4 (R-19, FR-012).
//!
//! Зачем свой разбор вместо FFmpeg. Чтобы показать разрешение, длительность и кодеки
//! файла, лежащего на сервере, нужно прочитать несколько сотен килобайт его начала —
//! не скачивать целиком и не гонять внешнюю программу. Для файла, подготовленного
//! нашим процессом (`-movflags +faststart`), заголовок как раз в начале и лежит.
//!
//! Что делает этот разбор, когда заголовка в начале нет: **не гадает**. Возвращает
//! «неизвестно» и признак, что файл не соответствует целевому формату (FR-012).
//! Такой файл зритель начнёт смотреть только после скачивания хвоста, и знать об этом
//! пользователю важнее, чем увидеть разрешение.
//!
//! Разбор идёт по недоверенным данным: файл на сервере мог оказаться каким угодно.
//! Поэтому здесь нет ни одного обращения по индексу без проверки границ — только
//! функции, возвращающие `Option`, и ограничение глубины вложенности.

/// Насколько глубоко готовы спускаться по вложенным атомам.
///
/// Реальная глубина до кодека — `moov/trak/mdia/minf/stbl/stsd` — шесть уровней.
/// Предел защищает от файла, собранного так, чтобы разбор ушёл вглубь навсегда.
const MAX_DEPTH: usize = 8;

/// Сколько байт заголовка просить у сервера при первой попытке.
///
/// У файла с `moov` в начале заголовок обычно укладывается в сотни килобайт: он
/// растёт с числом кадров. Если не хватит, разбор скажет, сколько нужно ровно
/// (см. [`MoovOutcome::NeedMoreBytes`]), и второй попытки хватит наверняка.
pub const SUGGESTED_HEAD_BYTES: u64 = 512 * 1024;

/// Параметры файла, вычитанные из заголовка.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaParams {
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Средний битрейт: объём файла, делённый на длительность. Считается только
    /// когда известно и то и другое.
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

/// Чем кончился разбор.
#[derive(Debug, Clone, PartialEq)]
pub enum MoovOutcome {
    /// Заголовок найден в начале и разобран.
    Parsed(MediaParams),
    /// Заголовок есть, но лежит после данных: файл не подготовлен к раздаче.
    /// Параметры остаются неизвестными — вычитывать их значило бы качать файл целиком.
    MoovAfterData,
    /// Прочитанного куска не хватило. `need` — сколько байт от начала файла нужно,
    /// чтобы продолжить.
    NeedMoreBytes { need: u64 },
    /// Это не MP4.
    NotMp4,
}

impl MoovOutcome {
    /// Значение поля `faststart_ok` файла раздачи.
    ///
    /// `None` — вопрос ещё открыт: данных не хватило или это вообще не MP4,
    /// и объявлять файл негодным не за что.
    pub fn faststart_ok(&self) -> Option<bool> {
        match self {
            Self::Parsed(_) => Some(true),
            Self::MoovAfterData => Some(false),
            Self::NeedMoreBytes { .. } | Self::NotMp4 => None,
        }
    }

    pub fn params(&self) -> Option<&MediaParams> {
        match self {
            Self::Parsed(p) => Some(p),
            _ => None,
        }
    }
}

/// Разобрать начало файла MP4.
///
/// `head` — прочитанные байты от начала файла. `file_size` — полный размер файла,
/// если известен: без него не посчитать средний битрейт, но всё остальное разберётся.
pub fn parse(head: &[u8], file_size: Option<u64>) -> MoovOutcome {
    let mut offset: usize = 0;
    let mut looks_like_mp4 = false;

    loop {
        let header = match BoxHeader::read(head, offset) {
            Ok(h) => h,
            // Данных не хватило на сам заголовок. Просить ещё имеет смысл только
            // если начало уже опознано как MP4.
            Err(HeaderError::Truncated) if looks_like_mp4 => {
                return need_more(offset as u64 + 16, file_size)
            }
            // Заголовок противоречив: длина меньше его самого. Дозапрос не поможет —
            // то, что уже прочитано, не сходится само с собой.
            Err(_) => return MoovOutcome::NotMp4,
        };

        if !looks_like_mp4 {
            // Первый атом обязан быть одним из тех, с которых начинается MP4.
            // Одной проверки «имя из печатаемых знаков» мало: у страницы с ошибкой
            // сервера («<!DOCTYPE html>») на месте имени оказывается «CTYP», и без
            // этого списка она сошла бы за видео с атомом на гигабайт.
            if !header.starts_a_file() {
                return MoovOutcome::NotMp4;
            }
            looks_like_mp4 = true;
        } else if !header.type_is_plausible() {
            return MoovOutcome::NotMp4;
        }

        match &header.typ {
            b"moov" => {
                let Some(payload) = header.payload(head) else {
                    // Атом начался, но кончается за пределами прочитанного. Мы точно
                    // знаем, сколько нужно, — просим ровно столько.
                    return need_more(offset as u64 + header.total_len, file_size);
                };
                let params = parse_moov(payload, file_size);
                return MoovOutcome::Parsed(params);
            }
            // Данные пошли раньше заголовка — файл не подготовлен к раздаче.
            // Дальше искать незачем: ответ уже известен, а `mdat` может быть
            // на гигабайты, и дочитывать его ради заголовка бессмысленно.
            b"mdat" => return MoovOutcome::MoovAfterData,
            _ => {}
        }

        let Some(next) = header.next_offset(offset) else {
            return MoovOutcome::NotMp4;
        };
        if next <= offset {
            // Атом нулевой длины: дальше разбор не сдвинется никогда.
            return MoovOutcome::NotMp4;
        }
        if next >= head.len() {
            return need_more(next as u64 + 16, file_size);
        }
        offset = next;
    }
}

/// Запросить недостающие байты — но только если они в файле вообще есть.
///
/// Без этой проверки разбор просил бы данные за концом файла, читающий слой отдавал
/// бы тот же кусок, а разбор просил бы снова: вечный круг на файле, у которого
/// заголовка нет вовсе. Когда просить больше нечего, ответ честный: это не то видео,
/// с которым мы умеем работать.
fn need_more(need: u64, file_size: Option<u64>) -> MoovOutcome {
    match file_size {
        Some(size) if need > size => MoovOutcome::NotMp4,
        _ => MoovOutcome::NeedMoreBytes { need },
    }
}

/// Сведения об одной дорожке.
#[derive(Debug, Default)]
struct Track {
    handler: Option<[u8; 4]>,
    codec: Option<String>,
    /// Разрешение из описания образца — то, в чём кадр закодирован.
    coded_size: Option<(u32, u32)>,
    /// Разрешение из заголовка дорожки — то, как её показывать.
    display_size: Option<(u32, u32)>,
    timescale: Option<u32>,
    duration: Option<u64>,
}

impl Track {
    fn is_video(&self) -> bool {
        self.handler.as_ref().is_some_and(|h| h == b"vide")
    }

    fn is_audio(&self) -> bool {
        self.handler.as_ref().is_some_and(|h| h == b"soun")
    }

    fn seconds(&self) -> Option<f64> {
        let ts = self.timescale.filter(|t| *t > 0)?;
        let d = self
            .duration
            .filter(|d| *d > 0 && *d != u64::from(u32::MAX))?;
        Some(d as f64 / f64::from(ts))
    }
}

fn parse_moov(payload: &[u8], file_size: Option<u64>) -> MediaParams {
    let mut params = MediaParams::default();
    let mut movie_seconds: Option<f64> = None;
    let mut tracks: Vec<Track> = Vec::new();

    walk(payload, 0, |typ, body| match typ {
        b"mvhd" => {
            movie_seconds = parse_mvhd(body);
        }
        b"trak" => {
            let mut track = Track::default();
            parse_trak(body, &mut track, 1);
            tracks.push(track);
        }
        _ => {}
    });

    let video = tracks.iter().find(|t| t.is_video());
    let audio = tracks.iter().find(|t| t.is_audio());

    if let Some(v) = video {
        // Разрешение берём из описания образца: это то, во что кадр закодирован,
        // и именно с ним сравниваются ступени набора качеств. Заголовок дорожки
        // хранит размер для показа, который у анаморфного видео отличается.
        let (w, h) = v.coded_size.or(v.display_size).unwrap_or((0, 0));
        if w > 0 && h > 0 {
            params.width = Some(w);
            params.height = Some(h);
        }
        params.video_codec = v.codec.clone();
    }
    if let Some(a) = audio {
        params.audio_codec = a.codec.clone();
    }

    // Длительность фильма — из заголовка фильма; если её там нет (встречается
    // «неизвестно»), берём самую длинную дорожку.
    params.duration_s = movie_seconds.or_else(|| {
        tracks
            .iter()
            .filter_map(Track::seconds)
            .fold(None, |acc: Option<f64>, s| {
                Some(acc.map_or(s, |a| a.max(s)))
            })
    });

    if let (Some(size), Some(seconds)) = (file_size, params.duration_s) {
        if seconds > 0.0 && size > 0 {
            let bps = (size as f64 * 8.0 / seconds).round();
            if bps.is_finite() && bps >= 0.0 {
                params.bitrate_bps = Some(bps as u64);
            }
        }
    }

    params
}

fn parse_mvhd(body: &[u8]) -> Option<f64> {
    let version = *body.first()?;
    let (timescale, duration) = if version == 1 {
        (be_u32(body, 20)?, be_u64(body, 24)?)
    } else {
        (be_u32(body, 12)?, u64::from(be_u32(body, 16)?))
    };
    if timescale == 0 || duration == 0 || duration == u64::from(u32::MAX) || duration == u64::MAX {
        return None;
    }
    Some(duration as f64 / f64::from(timescale))
}

fn parse_trak(body: &[u8], track: &mut Track, depth: usize) {
    walk(body, depth, |typ, inner| match typ {
        b"tkhd" => {
            track.display_size = parse_tkhd_size(inner);
        }
        b"mdia" => {
            walk(inner, depth + 1, |t, b| match t {
                b"mdhd" => {
                    if let Some((ts, d)) = parse_mdhd(b) {
                        track.timescale = Some(ts);
                        track.duration = Some(d);
                    }
                }
                b"hdlr" => {
                    track.handler = parse_hdlr(b);
                }
                b"minf" => {
                    walk(b, depth + 2, |t2, b2| {
                        if t2 == b"stbl" {
                            walk(b2, depth + 3, |t3, b3| {
                                if t3 == b"stsd" {
                                    if let Some((codec, size)) = parse_stsd(b3) {
                                        track.codec = Some(codec);
                                        track.coded_size = size;
                                    }
                                }
                            });
                        }
                    });
                }
                _ => {}
            });
        }
        _ => {}
    });
}

fn parse_tkhd_size(body: &[u8]) -> Option<(u32, u32)> {
    let version = *body.first()?;
    // Поля до матрицы преобразования: у первой версии времена и длительность
    // по четыре байта, у второй — по восемь.
    let base = if version == 1 { 4 + 32 } else { 4 + 20 };
    let after_matrix = base + 8 + 2 + 2 + 2 + 2 + 36;
    let width = be_u32(body, after_matrix)?;
    let height = be_u32(body, after_matrix + 4)?;
    // Значения записаны с дробной частью в младших шестнадцати битах.
    let (w, h) = (width >> 16, height >> 16);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn parse_mdhd(body: &[u8]) -> Option<(u32, u64)> {
    let version = *body.first()?;
    if version == 1 {
        Some((be_u32(body, 20)?, be_u64(body, 24)?))
    } else {
        Some((be_u32(body, 12)?, u64::from(be_u32(body, 16)?)))
    }
}

fn parse_hdlr(body: &[u8]) -> Option<[u8; 4]> {
    let slice = body.get(8..12)?;
    let mut out = [0u8; 4];
    out.copy_from_slice(slice);
    Some(out)
}

/// Кодек и, для видео, закодированное разрешение — из первого описания образца.
fn parse_stsd(body: &[u8]) -> Option<(String, Option<(u32, u32)>)> {
    // version+flags (4), entry_count (4), дальше первая запись.
    let entry = 8usize;
    let entry_size = be_u32(body, entry)?;
    if entry_size < 8 {
        return None;
    }
    let format = body.get(entry + 4..entry + 8)?;
    let codec = codec_name(format);

    // Внутри записи для видео: reserved(6) + data_reference_index(2) +
    // pre_defined(2) + reserved(2) + pre_defined(12) = 24 байта до размеров.
    let size_at = entry + 8 + 24;
    let coded = match (be_u16(body, size_at), be_u16(body, size_at + 2)) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((u32::from(w), u32::from(h))),
        _ => None,
    };

    Some((codec, coded))
}

/// Человеческое имя кодека по четырёхбуквенному коду формата.
///
/// Незнакомый код возвращается как есть: показать пользователю «hvc1» честнее,
/// чем промолчать, — по нему хотя бы можно найти, что это.
fn codec_name(format: &[u8]) -> String {
    match format {
        b"avc1" | b"avc3" => String::from("h264"),
        b"hev1" | b"hvc1" => String::from("h265"),
        b"av01" => String::from("av1"),
        b"vp09" => String::from("vp9"),
        b"mp4a" => String::from("aac"),
        b"ac-3" => String::from("ac3"),
        b"ec-3" => String::from("eac3"),
        b"Opus" | b"opus" => String::from("opus"),
        b"fLaC" => String::from("flac"),
        b".mp3" | b"mp3 " => String::from("mp3"),
        other => String::from_utf8_lossy(other).trim().to_owned(),
    }
}

/// Обойти вложенные атомы, вызывая обработчик для каждого.
///
/// Молча останавливается на первом же непонятном месте: испорченный хвост не должен
/// отменять то, что уже прочитано из начала.
fn walk<F>(data: &[u8], depth: usize, mut visit: F)
where
    F: FnMut(&[u8; 4], &[u8]),
{
    if depth > MAX_DEPTH {
        return;
    }
    let mut offset = 0usize;
    while let Ok(header) = BoxHeader::read(data, offset) {
        if !header.type_is_plausible() {
            return;
        }
        if let Some(payload) = header.payload(data) {
            visit(&header.typ, payload);
        }
        let Some(next) = header.next_offset(offset) else {
            return;
        };
        if next <= offset || next >= data.len() {
            return;
        }
        offset = next;
    }
}

/// Почему заголовок атома не прочитан.
///
/// Разница между двумя случаями решает, дозапрашивать ли данные с сервера:
/// оборванному куску это поможет, противоречивому — нет никогда.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderError {
    /// Байтов не хватило на сам заголовок.
    Truncated,
    /// Заголовок не сходится сам с собой: длина меньше его собственной.
    Malformed,
}

/// Имена атомов, с которых может начинаться файл MP4.
///
/// По стандарту первым обязан идти `ftyp`, но встречаются файлы и без него,
/// поэтому список чуть шире — ровно на то, что действительно бывает в начале.
const FILE_STARTERS: [&[u8; 4]; 6] = [b"ftyp", b"styp", b"moov", b"free", b"skip", b"mdat"];

/// Заголовок атома: имя и длина вместе с самим заголовком.
struct BoxHeader {
    typ: [u8; 4],
    /// Длина заголовка: восемь байт обычно, шестнадцать при длине в восемь байт.
    header_len: usize,
    /// Полная длина атома вместе с заголовком.
    total_len: u64,
    /// Смещение начала атома в исходных данных.
    start: usize,
}

impl BoxHeader {
    fn read(data: &[u8], offset: usize) -> Result<Self, HeaderError> {
        let size32 = be_u32(data, offset).ok_or(HeaderError::Truncated)?;
        let mut typ = [0u8; 4];
        typ.copy_from_slice(
            data.get(offset + 4..offset + 8)
                .ok_or(HeaderError::Truncated)?,
        );

        let (header_len, total_len) = match size32 {
            // Единица значит, что настоящая длина записана следом восемью байтами.
            1 => (
                16usize,
                be_u64(data, offset + 8).ok_or(HeaderError::Truncated)?,
            ),
            // Ноль значит «до конца файла».
            0 => (8usize, (data.len() - offset) as u64),
            n if n < 8 => return Err(HeaderError::Malformed),
            n => (8usize, u64::from(n)),
        };

        if total_len < header_len as u64 {
            return Err(HeaderError::Malformed);
        }

        Ok(Self {
            typ,
            header_len,
            total_len,
            start: offset,
        })
    }

    /// Может ли атом с таким именем стоять в начале файла.
    fn starts_a_file(&self) -> bool {
        FILE_STARTERS.contains(&&self.typ)
    }

    /// Похоже ли имя атома на имя, а не на случайные байты.
    fn type_is_plausible(&self) -> bool {
        self.typ.iter().all(
            |b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b' ' | b'-' | b'.' | 0xA9),
        )
    }

    /// Содержимое атома, если оно целиком есть в прочитанном куске.
    fn payload<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        let from = self.start.checked_add(self.header_len)?;
        let to = usize::try_from(self.total_len)
            .ok()
            .and_then(|len| self.start.checked_add(len))?;
        data.get(from..to)
    }

    fn next_offset(&self, offset: usize) -> Option<usize> {
        usize::try_from(self.total_len)
            .ok()
            .and_then(|len| offset.checked_add(len))
    }
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn be_u64(data: &[u8], at: usize) -> Option<u64> {
    let bytes = data.get(at..at + 8)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(bytes);
    Some(u64::from_be_bytes(out))
}
