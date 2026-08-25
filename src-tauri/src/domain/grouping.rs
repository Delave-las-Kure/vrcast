//! T033 — восстановление группировки по именам (FR-015).
//!
//! Задача: на сервере лежат файлы, залитые мимо приложения — скриптами, руками,
//! прошлым способом работы. Описи для них нет. Приложение обязано не прятать их,
//! а показать и по возможности предложить, что чем является.
//!
//! Опора — соглашение об именах, по которому работают существующие скрипты:
//! варианты одного произведения называются `<имя>_<битрейт>.mp4`
//! (`Backrooms_10.mp4`, `Backrooms_22.mp4`, `Backrooms_35.mp4`), а набор качеств
//! лежит в каталоге `<имя>/`.
//!
//! **Предложение, а не решение.** Ничего не привязывается автоматически: угаданная
//! связь, записанная в опись без спроса, потом расходится с тем, что человек имел
//! в виду, и разбираться в этом тяжелее, чем сгруппировать вручную.

/// Предложенная группа файлов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedGroup {
    /// Общая часть имени — готовый `slug`, если пользователь согласится.
    pub key: String,
    /// Название для показа: та же общая часть, приведённая к читаемому виду.
    pub suggested_title: String,
    /// Пути файлов, относительно каталога видео. Порядок исходный.
    pub files: Vec<String>,
    /// Почему файлы сведены вместе — показывается пользователю, чтобы он мог
    /// не гадать, откуда взялось предложение.
    pub reason: GroupReason,
}

/// Основание, по которому файлы сведены в группу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupReason {
    /// Лежат в одном каталоге — обычно это набор качеств.
    SameDirectory,
    /// Имена различаются только числом после подчёркивания — варианты по битрейту.
    BitrateVariants,
}

impl GroupReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SameDirectory => "лежат в одном каталоге",
            Self::BitrateVariants => "варианты одного файла по битрейту",
        }
    }
}

/// Итог разбора.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suggestion {
    /// Файлы, которые удалось свести вместе.
    pub groups: Vec<SuggestedGroup>,
    /// Файлы, для которых оснований не нашлось. Показываются по одному —
    /// но показываются обязательно (FR-015).
    pub singles: Vec<String>,
}

impl Suggestion {
    /// Сколько всего файлов учтено. Служит проверкой полноты: ни один файл
    /// не должен потеряться между группами и одиночками (FR-015, SC про полноту).
    pub fn total_files(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum::<usize>() + self.singles.len()
    }
}

/// Предложить группировку для файлов, о которых опись ничего не знает.
///
/// Порядок входа сохраняется в выходе: пользователь видит список в том же порядке,
/// что и на сервере, и ему не приходится искать знакомое имя заново.
pub fn suggest(paths: &[String]) -> Suggestion {
    // Ключ → (основание, номера файлов). Порядок ключей — по первому появлению.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, (GroupReason, Vec<String>)> =
        std::collections::HashMap::new();
    let mut singles: Vec<String> = Vec::new();

    for path in paths {
        match classify(path) {
            Some((key, reason)) => {
                let entry = buckets.entry(key.clone()).or_insert_with(|| {
                    order.push(key.clone());
                    (reason, Vec::new())
                });
                entry.1.push(path.clone());
            }
            None => singles.push(path.clone()),
        }
    }

    let mut groups = Vec::new();
    for key in order {
        let Some((reason, files)) = buckets.remove(&key) else {
            continue;
        };
        // Один файл — это не группа. Единственный `Backrooms_22.mp4` без соседей
        // ничего не доказывает, и заводить под него медиа самовольно не за что.
        // Каталог — другое дело: он и есть заявленная связь.
        if files.len() == 1 && reason == GroupReason::BitrateVariants {
            singles.extend(files);
            continue;
        }
        groups.push(SuggestedGroup {
            suggested_title: readable_title(&key),
            key,
            files,
            reason,
        });
    }

    Suggestion { groups, singles }
}

/// К какой группе отнести путь и почему.
fn classify(path: &str) -> Option<(String, GroupReason)> {
    let normalized = path.trim_matches('/');
    if normalized.is_empty() {
        return None;
    }

    // Путь с каталогом: каталог и есть заявленная связь.
    if let Some((dir, _rest)) = normalized.split_once('/') {
        if !dir.is_empty() {
            return Some((dir.to_owned(), GroupReason::SameDirectory));
        }
    }

    // Имя вида `<общая часть>_<число>.<расширение>`.
    let stem = normalized.rsplit_once('.').map_or(normalized, |(s, _)| s);
    let (prefix, tail) = stem.rsplit_once('_')?;
    if prefix.is_empty() || tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((prefix.to_owned(), GroupReason::BitrateVariants))
}

/// Читаемое название из общей части имени: разделители — в пробелы.
fn readable_title(key: &str) -> String {
    let spaced: String = key
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        key.to_owned()
    } else {
        collapsed
    }
}
