//! T031 — опись библиотеки `library.json` со счётчиком поколения
//! (`contracts/server-contract.md`, раздел «Опись библиотеки»).
//!
//! Счётчик поколения существует ради одного случая: два экземпляра приложения,
//! работающие с одним сервером. Порядок записи обязателен (R-10): прочитать
//! с поколением → изменить → записать рядом → атомарно заменить. Если поколение
//! на сервере успело измениться, запись **не выполняется** — иначе второй экземпляр
//! молча сотрёт работу первого.
//!
//! Здесь только разбор, сборка и правила. Сама запись на сервер — в `server::manifest_io`.

use super::media::{validate_slug, Media};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Опись библиотеки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Растёт на единицу при каждой записи. Ноль = описи ещё не было.
    pub generation: u64,
    #[serde(default)]
    pub media: Vec<Media>,
    /// Поля, которых это приложение не знает.
    ///
    /// Сохраняются при перезаписи намеренно: опись могла быть написана более новой
    /// версией приложения, и терять её данные нельзя (FR-131). Молча выбрасывать
    /// непонятое — самый тихий способ испортить чужие сведения.
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self::empty()
    }
}

/// Почему опись не удалось прочитать.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("опись не разобрать: {0}")]
    Malformed(String),
}

/// Что не так внутри описи. Опись живёт на сервере, её мог править человек.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestProblem {
    DuplicateId(String),
    DuplicateSlug(String),
    /// Один файл числится за двумя медиа — при удалении одного пропадёт и у другого.
    FileClaimedTwice {
        path: String,
        media: Vec<String>,
    },
    EmptyId,
    BadSlug {
        slug: String,
        reason: String,
    },
}

impl std::fmt::Display for ManifestProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "два медиа с одинаковым номером «{id}»"),
            Self::DuplicateSlug(slug) => {
                write!(f, "два медиа с одинаковым коротким именем «{slug}»")
            }
            Self::FileClaimedTwice { path, media } => write!(
                f,
                "файл «{path}» числится сразу за несколькими медиа: {}",
                media.join(", ")
            ),
            Self::EmptyId => f.write_str("у медиа пустой номер"),
            Self::BadSlug { slug, reason } => {
                write!(f, "короткое имя «{slug}» недопустимо: {reason}")
            }
        }
    }
}

impl Manifest {
    /// Пустая опись — то, с чего начинается сервер без библиотеки.
    pub fn empty() -> Self {
        Self {
            generation: 0,
            media: Vec::new(),
            extra: HashMap::new(),
        }
    }

    /// Разобрать содержимое `library.json`.
    ///
    /// Пустое содержимое — это отсутствующая опись, а не ошибка: на свежем сервере
    /// файла ещё нет, и падать тут значило бы объявить пустую библиотеку поломкой.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        if text.trim().is_empty() {
            return Ok(Self::empty());
        }
        serde_json::from_str(text).map_err(|e| ManifestError::Malformed(e.to_string()))
    }

    /// Собрать содержимое для записи на сервер.
    ///
    /// С отступами, потому что файл читают и правят люди — в том числе когда
    /// приложение недоступно, а разобраться надо.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{}"))
    }

    /// Опись для записи поверх прочитанной: поколение на единицу больше.
    ///
    /// Отдельный шаг, а не `+= 1` где придётся: увеличение поколения — это и есть
    /// заявка «я записываю поверх того, что прочитал», и делаться она должна в одном
    /// месте.
    pub fn prepared_for_write(&self) -> Self {
        let mut next = self.clone();
        next.generation = self.generation.saturating_add(1);
        next
    }

    /// Можно ли записывать: то ли поколение сейчас на сервере, что было прочитано.
    ///
    /// `base` — поколение, прочитанное перед изменением; `current` — то, что на
    /// сервере сейчас.
    pub fn write_allowed(base: u64, current: u64) -> bool {
        base == current
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.slug == slug)
    }

    pub fn find_by_id(&self, id: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Все файлы и описания наборов качеств, числящиеся за медиа.
    pub fn all_claimed_paths(&self) -> Vec<&str> {
        self.media
            .iter()
            .flat_map(|m| m.files.iter().chain(m.ladders.iter()))
            .map(String::as_str)
            .collect()
    }

    /// Свободно ли короткое имя (`slug` уникален в пределах сервера).
    ///
    /// `except_id` позволяет проверять при переименовании: медиа не конфликтует
    /// с самим собой.
    pub fn slug_available(&self, slug: &str, except_id: Option<&str>) -> bool {
        !self
            .media
            .iter()
            .any(|m| m.slug == slug && Some(m.id.as_str()) != except_id)
    }

    /// Проверить опись целиком. Возвращает **все** замечания.
    pub fn validate(&self) -> Result<(), Vec<ManifestProblem>> {
        let mut problems = Vec::new();
        let mut seen_ids: HashMap<&str, usize> = HashMap::new();
        let mut seen_slugs: HashMap<&str, usize> = HashMap::new();
        // Порядок владельцев сохраняем: сообщение об ошибке должно называть их
        // в том же порядке, что и опись, иначе оно меняется от запуска к запуску.
        let mut owners: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut owner_index: HashMap<&str, usize> = HashMap::new();

        for m in &self.media {
            if m.id.trim().is_empty() {
                problems.push(ManifestProblem::EmptyId);
            } else {
                *seen_ids.entry(m.id.as_str()).or_insert(0) += 1;
            }

            match validate_slug(&m.slug) {
                Ok(()) => {
                    *seen_slugs.entry(m.slug.as_str()).or_insert(0) += 1;
                }
                Err(e) => problems.push(ManifestProblem::BadSlug {
                    slug: m.slug.clone(),
                    reason: e.to_string(),
                }),
            }

            for path in m.files.iter().chain(m.ladders.iter()) {
                let idx = *owner_index.entry(path.as_str()).or_insert_with(|| {
                    owners.push((path.as_str(), Vec::new()));
                    owners.len() - 1
                });
                owners[idx].1.push(m.id.as_str());
            }
        }

        for id in sorted_duplicates(&seen_ids) {
            problems.push(ManifestProblem::DuplicateId(id.to_owned()));
        }
        for slug in sorted_duplicates(&seen_slugs) {
            problems.push(ManifestProblem::DuplicateSlug(slug.to_owned()));
        }
        for (path, claimants) in owners {
            if claimants.len() > 1 {
                problems.push(ManifestProblem::FileClaimedTwice {
                    path: path.to_owned(),
                    media: claimants.into_iter().map(str::to_owned).collect(),
                });
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Повторяющиеся ключи в устойчивом порядке: сообщения об ошибках не должны
/// меняться от запуска к запуску только потому, что обход словаря случаен.
fn sorted_duplicates<'a>(counts: &HashMap<&'a str, usize>) -> Vec<&'a str> {
    let mut dups: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(k, _)| *k)
        .collect();
    dups.sort_unstable();
    dups
}
