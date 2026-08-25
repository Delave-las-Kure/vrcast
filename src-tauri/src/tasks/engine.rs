//! T016, T019 — очередь задач, полосы по ресурсам, отмена и приостановка.
//!
//! Устройство подчинено трём требованиям, и каждое влияет на форму:
//!
//! - **Интерфейс остаётся отзывчивым** (FR-080, SC-009): вся работа идёт в исполнителе,
//!   а наружу уходит поток событий, а не запросы состояния.
//! - **Задачи переживают перезапуск** (FR-081): значимые сдвиги пишутся в базу, а
//!   застигнутые в работе при следующем старте становятся приостановленными, но никогда
//!   завершёнными (конституция, принцип III).
//! - **Отмена не считается выполненной, пока живо дерево процессов** (принцип III):
//!   состояние записывается только после того, как процессов не осталось.

use super::progress::ProgressThrottle;
use super::state::{Lane, LaneLimits, TaskKind, TaskState};
use super::store::{self, TaskRecord};
use crate::store::db::{Db, DbError};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("задача {0} не найдена")]
    NotFound(String),

    #[error("переход {from} → {to} для задачи {id} недопустим")]
    BadTransition {
        id: String,
        from: &'static str,
        to: &'static str,
    },

    #[error("задачу этого вида нельзя приостановить")]
    NotPausable,

    #[error("задача отменена")]
    Cancelled,

    #[error("{0}")]
    Failed(String),

    #[error(transparent)]
    Db(#[from] DbError),
}

pub type Result<T> = std::result::Result<T, TaskError>;

/// Как часто продвижение попадает в базу.
///
/// Три секунды — это про то, сколько работы не жалко переспросить у человека после
/// перезапуска, а не про плавность показа: плавность даёт поток событий.
const PROGRESS_PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Событие о задаче, уходящее в интерфейс.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEvent {
    Progress {
        id: String,
        state: TaskState,
        progress: f64,
        stage: Option<String>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
    },
    Done {
        id: String,
        state: TaskState,
        error: Option<String>,
    },
}

/// Ручки управления живой задачей.
struct LiveTask {
    kind: TaskKind,
    state: TaskState,
    cancel: CancellationToken,
    /// Взведён, пока задача приостановлена.
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    /// Место в очереди: меньше — раньше (FR-083).
    position: i64,
}

/// То, что видит выполняющаяся задача.
///
/// Через него она сообщает о продвижении и узнаёт, не пора ли остановиться. Ничего
/// другого задаче знать не нужно — ни про базу, ни про очередь.
#[derive(Clone)]
pub struct TaskContext {
    pub id: String,
    cancel: CancellationToken,
    paused: Arc<Mutex<bool>>,
    resume: Arc<Notify>,
    throttle: Arc<ProgressThrottle>,
    /// Отдельный клапан — для записи продвижения в базу.
    ///
    /// Не тот же, что у событий: события идут в память четыре раза в секунду,
    /// а запись на диск с такой частотой ради показателя не нужна никому.
    persist_throttle: Arc<ProgressThrottle>,
    events: broadcast::Sender<TaskEvent>,
    db: Arc<Db>,
}

impl TaskContext {
    /// Отменена ли задача. Проверять в местах, где можно остановиться без вреда.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Токен отмены — чтобы передать его в ожидание ввода-вывода.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Прерваться, если задачу отменили.
    pub fn bail_if_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(TaskError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Дождаться продолжения, если задача приостановлена.
    ///
    /// Вызывать между единицами работы: между кусками передачи, между шагами
    /// развёртывания. Приостановка вступит в силу на ближайшей такой точке.
    /// Отмена тоже будит: после возврата вызывающий обязан проверить `is_cancelled`.
    pub async fn wait_while_paused(&self) {
        loop {
            // Подписка на «продолжить» оформляется ДО чтения флага: notify_waiters
            // не сохраняет разрешения для ещё не подписанных, и иначе «продолжить»,
            // нажатое в щели между чтением флага и засыпанием, потерялось бы навсегда.
            let resumed = self.resume.notified();

            if self.is_cancelled() || !*self.paused.lock().unwrap_or_else(|e| e.into_inner()) {
                return;
            }

            // Ждать одного лишь «продолжить» нельзя: при отмене флаг приостановки
            // не сбрасывается, и задача, разбуженная отменой, тут же уснула бы снова.
            tokio::select! {
                _ = resumed => {}
                _ = self.cancel.cancelled() => return,
            }
        }
    }

    /// Сообщить о продвижении. Частота ограничена (T020).
    pub fn report(&self, progress: f64, stage: impl Into<String>) {
        self.report_full(progress, Some(stage.into()), None, None, false);
    }

    /// Сообщить о продвижении с показателями передачи.
    pub fn report_transfer(&self, progress: f64, speed_bps: i64, eta_s: i64) {
        self.report_full(progress, None, Some(speed_bps), Some(eta_s), false);
    }

    /// Сообщение, которое обязано пройти независимо от частоты: смена этапа, конец работы.
    pub fn report_important(&self, progress: f64, stage: impl Into<String>) {
        self.report_full(progress, Some(stage.into()), None, None, true);
    }

    fn report_full(
        &self,
        progress: f64,
        stage: Option<String>,
        speed_bps: Option<i64>,
        eta_s: Option<i64>,
        important: bool,
    ) {
        if !self.throttle.allow(important) {
            return;
        }
        let _ = self.events.send(TaskEvent::Progress {
            id: self.id.clone(),
            state: TaskState::Running,
            progress: progress.clamp(0.0, 1.0),
            stage,
            speed_bps,
            eta_s,
        });
    }

    /// Запомнить продвижение так, чтобы оно пережило закрытие приложения.
    ///
    /// Событиями продвижение расходится по интерфейсу четыре раза в секунду, но живёт
    /// только в памяти. В базу оно пишется много реже — раз в несколько секунд:
    /// точность до секунды здесь никому не нужна, а держать диск занятым ради неё
    /// незачем. Задача сама решает, когда звать: слой задач не знает, что считать
    /// продвижением.
    ///
    /// Неудача записи проглатывается намеренно: показатель — не работа, и ронять
    /// из-за него многочасовую передачу нельзя.
    pub fn save_progress(&self, progress: f64) {
        if !self.persist_throttle.allow(false) {
            return;
        }
        if let Err(e) = store::save_progress(&self.db, &self.id, progress) {
            tracing::debug!(id = %self.id, error = %e, "продвижение не записано");
        }
    }

    /// Записать позицию возобновления.
    ///
    /// Это единственное, что имеет смысл писать в базу часто: без него прерванная
    /// передача начнётся заново. Запись точечная — см. `store::save_resume_token`.
    pub fn save_resume_token(&self, token: &str) -> Result<()> {
        store::save_resume_token(&self.db, &self.id, token)?;
        Ok(())
    }

    /// Прочитать позицию возобновления, оставленную прошлым запуском.
    pub fn resume_token(&self) -> Result<Option<String>> {
        Ok(store::get(&self.db, &self.id)?.and_then(|r| r.resume_token))
    }
}

/// Итог попытки занять место в полосе.
enum ClaimOutcome {
    /// Место занято, задача стала выполняющейся.
    Started,
    /// Полоса заполнена — подождать и попробовать снова.
    Busy,
    /// Задачи больше нет среди живых (снята или завершена) — не запускать.
    Gone,
}

/// Механизм задач.
#[derive(Clone)]
pub struct TaskEngine {
    db: Arc<Db>,
    live: Arc<Mutex<HashMap<String, LiveTask>>>,
    limits: LaneLimits,
    events: broadcast::Sender<TaskEvent>,
    /// Номер, который получит следующая поставленная задача.
    next_position: Arc<std::sync::atomic::AtomicI64>,
}

impl TaskEngine {
    pub fn new(db: Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(256);
        // Отсчёт продолжается с того места, где кончил прошлый запуск: иначе новая
        // задача получила бы номер, уже занятый лежащей в базе, и встала бы в середину
        // чужой очереди.
        let next = store::max_queue_order(&db).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "не прочитать порядок очереди — начинаем с нуля");
            0
        }) + 1;
        Self {
            db,
            live: Arc::new(Mutex::new(HashMap::new())),
            limits: LaneLimits::default(),
            events,
            next_position: Arc::new(std::sync::atomic::AtomicI64::new(next)),
        }
    }

    pub fn with_limits(mut self, limits: LaneLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Подписаться на события задач.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// Разобрать состояние после запуска приложения (T017).
    pub fn recover_after_start(&self) -> Result<store::RecoveryReport> {
        Ok(store::recover_after_start(&self.db)?)
    }

    /// Сколько задач сейчас занимает полосу.
    pub fn running_in_lane(&self, lane: Lane) -> usize {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.values()
            .filter(|t| t.state.occupies_lane() && t.kind.lane() == lane)
            .count()
    }

    /// Есть ли место в полосе для задачи этого вида.
    pub fn has_room_for(&self, kind: TaskKind) -> bool {
        let lane = kind.lane();
        self.running_in_lane(lane) < self.limits.for_lane(lane)
    }

    /// Поставить задачу и запустить её, когда освободится место в полосе.
    ///
    /// Возвращает идентификатор сразу: команда не блокируется на длительной работе
    /// (FR-080, договор слоя команд).
    pub async fn submit<F, Fut>(
        &self,
        kind: TaskKind,
        server_id: Option<String>,
        work: F,
    ) -> Result<String>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), String>> + Send + 'static,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let mut record = TaskRecord::new(id.clone(), kind, server_id);
        record.queue_order = self
            .next_position
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        store::upsert(&self.db, &record)?;
        self.start(
            id.clone(),
            kind,
            TaskState::Queued,
            record.queue_order,
            work,
        );
        Ok(id)
    }

    /// Вернуть к жизни задачу прошлого запуска, не создавая новой.
    ///
    /// Нужно ради FR-031: заливка обязана продолжаться после закрытия и повторного
    /// запуска приложения. Без этого задача видна в списке приостановленной, но
    /// продолжить её нечем — рабочая часть живёт только в памяти и умирает вместе
    /// с приложением. Номер сохраняется прежний: у неё та же позиция возобновления,
    /// та же запись в базе и то же место в глазах человека.
    ///
    /// Задача поднимается **приостановленной**: она ждёт, пока человек скажет
    /// «продолжить». Самовольно возобновлять многочасовую передачу при запуске
    /// приложения нельзя — человек мог закрыть его именно чтобы она прекратилась.
    pub fn resubmit_paused<F, Fut>(&self, id: &str, work: F) -> Result<()>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), String>> + Send + 'static,
    {
        let record = store::get(&self.db, id)?.ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
        if record.state.is_final() {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: record.state.as_str(),
                to: TaskState::Paused.as_str(),
            });
        }
        if self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
        {
            // Уже жива — поднимать второй раз нельзя: получатся две работы
            // с одним номером.
            return Ok(());
        }

        store::save_state(&self.db, id, TaskState::Paused, None)?;
        self.start(
            id.to_owned(),
            record.kind,
            TaskState::Paused,
            record.queue_order,
            work,
        );
        Ok(())
    }

    /// Общая часть постановки: завести живую задачу и запустить её работу.
    fn start<F, Fut>(&self, id: String, kind: TaskKind, initial: TaskState, position: i64, work: F)
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = std::result::Result<(), String>> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        let paused = Arc::new(Mutex::new(initial == TaskState::Paused));
        let resume = Arc::new(Notify::new());
        let throttle = Arc::new(ProgressThrottle::default());

        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            live.insert(
                id.clone(),
                LiveTask {
                    kind,
                    state: initial,
                    cancel: cancel.clone(),
                    paused: paused.clone(),
                    resume: resume.clone(),
                    throttle: throttle.clone(),
                    position,
                },
            );
        }

        let ctx = TaskContext {
            id: id.clone(),
            cancel: cancel.clone(),
            paused,
            resume,
            throttle,
            persist_throttle: Arc::new(ProgressThrottle::new(PROGRESS_PERSIST_INTERVAL)),
            events: self.events.clone(),
            db: self.db.clone(),
        };

        let engine = self.clone();
        let task_id = id.clone();
        let paused_flag = ctx.paused.clone();
        let resume_signal = ctx.resume.clone();
        tokio::spawn(async move {
            // Поднятая после перезапуска задача ждёт человека и **не занимает полосу**:
            // иначе она заняла бы место, ничего не делая, и вторая такая же не смогла
            // бы начаться. Самовольно продолжать многочасовую передачу при запуске
            // приложения тоже нельзя — его могли закрыть именно ради её прекращения.
            loop {
                let resumed = resume_signal.notified();
                if cancel.is_cancelled() {
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                if !*paused_flag.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }
                tokio::select! {
                    _ = resumed => {}
                    _ = cancel.cancelled() => {
                        engine.finish(&task_id, TaskState::Cancelled, None);
                        return;
                    }
                }
            }

            // Ждём места в полосе. Отмена работает и здесь: стоящую в очереди задачу
            // можно снять, не дожидаясь её запуска.
            loop {
                if cancel.is_cancelled() {
                    engine.finish(&task_id, TaskState::Cancelled, None);
                    return;
                }
                match engine.try_claim_lane(&task_id) {
                    ClaimOutcome::Started => break,
                    ClaimOutcome::Busy => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    ClaimOutcome::Gone => return,
                }
            }

            let outcome = work(ctx).await;

            // Отмена важнее исхода работы: задача, снятая пользователем, не «упала».
            if cancel.is_cancelled() {
                engine.finish(&task_id, TaskState::Cancelled, None);
                return;
            }

            match outcome {
                Ok(()) => engine.finish(&task_id, TaskState::Completed, None),
                Err(e) => engine.finish(&task_id, TaskState::Failed, Some(e)),
            }
        });
    }

    /// Переставить задачи в очереди (FR-083).
    ///
    /// `ordered` — номера задач в желаемом порядке, как их видит человек в списке.
    /// Переставляются только те из них, что **ждут своей очереди**: выполняющуюся
    /// задача перестановка не трогает, потому что прервать её ради изменения порядка
    /// значило бы выбросить уже сделанную работу. Занятые места перераспределяются
    /// между собой, так что задачи, которых в списке нет, остаются там, где стояли.
    ///
    /// Задачи, успевшие начаться или закончиться между показом списка и нажатием,
    /// пропускаются молча: список у человека на экране всегда чуть отстаёт, и отказ
    /// от всей перестановки из-за одной такой задачи был бы наказанием за чужую
    /// расторопность. Возвращается, сколько задач действительно переставлено.
    pub fn reorder_queue(&self, ordered: &[String]) -> Result<usize> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());

        // Берём только тех, кто ещё ждёт, сохраняя порядок из заявки.
        let ожидающие: Vec<&String> = ordered
            .iter()
            .filter(|id| {
                live.get(id.as_str())
                    .is_some_and(|t| t.state == TaskState::Queued)
            })
            .collect();
        if ожидающие.len() < 2 {
            // Переставлять нечего: одна задача или ни одной.
            return Ok(0);
        }

        // Места, которые они занимают сейчас, — их и раздаём в новом порядке.
        // Так задачи, не попавшие в заявку, не сдвигаются ни на шаг.
        let mut места: Vec<i64> = ожидающие
            .iter()
            .filter_map(|id| live.get(id.as_str()).map(|t| t.position))
            .collect();
        места.sort_unstable();

        let mut записать: Vec<(String, i64)> = Vec::with_capacity(ожидающие.len());
        for (id, место) in ожидающие.iter().zip(места.iter()) {
            записать.push(((*id).clone(), *место));
        }
        for (id, место) in &записать {
            if let Some(t) = live.get_mut(id.as_str()) {
                t.position = *место;
            }
        }
        drop(live);

        // В базу — чтобы порядок пережил перезапуск приложения.
        for (id, место) in &записать {
            if let Err(e) = store::save_queue_order(&self.db, id, *место) {
                tracing::warn!(id, error = %e, "порядок очереди не записан");
            }
        }
        Ok(записать.len())
    }

    /// Номера ждущих задач в том порядке, в каком они пойдут в работу.
    pub fn queue_order(&self) -> Vec<String> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let mut ждущие: Vec<(&String, i64)> = live
            .iter()
            .filter(|(_, t)| t.state == TaskState::Queued)
            .map(|(id, t)| (id, t.position))
            .collect();
        ждущие.sort_by_key(|(_, position)| *position);
        ждущие.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// Отменить задачу.
    ///
    /// Токен взводится сразу, но состояние записывается только когда работа
    /// действительно прекратилась — включая дерево процессов (принцип III).
    pub fn cancel(&self, id: &str) -> Result<()> {
        let token = {
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            match live.get(id) {
                Some(t) => {
                    // Приостановленная задача не проснётся сама — будим, чтобы она
                    // увидела отмену.
                    t.resume.notify_waiters();
                    Some(t.cancel.clone())
                }
                None => None,
            }
        };

        match token {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            // Задачи нет среди живых: она осталась от прошлого запуска и никем
            // не поднята. Останавливать нечего, но решение человека записать надо —
            // иначе она навсегда останется в списке приостановленной, и снять её
            // будет нечем.
            None => {
                let record =
                    store::get(&self.db, id)?.ok_or_else(|| TaskError::NotFound(id.to_owned()))?;
                if record.state.is_final() {
                    // Повтор безопасен (конституция, принцип V).
                    return Ok(());
                }
                store::save_state(&self.db, id, TaskState::Cancelled, None)?;
                let _ = self.events.send(TaskEvent::Done {
                    id: id.to_owned(),
                    state: TaskState::Cancelled,
                    error: None,
                });
                Ok(())
            }
        }
    }

    /// Приостановить задачу.
    pub fn pause(&self, id: &str) -> Result<()> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let t = live
            .get_mut(id)
            .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;

        if t.kind.pause_kind() == super::state::PauseKind::NotPausable {
            return Err(TaskError::NotPausable);
        }
        if !t.state.can_transition_to(TaskState::Paused) {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: t.state.as_str(),
                to: TaskState::Paused.as_str(),
            });
        }

        *t.paused.lock().unwrap_or_else(|e| e.into_inner()) = true;
        t.state = TaskState::Paused;
        drop(live);
        self.persist_state(id, TaskState::Paused, None);
        Ok(())
    }

    /// Продолжить приостановленную задачу.
    pub fn resume(&self, id: &str) -> Result<()> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let t = live
            .get_mut(id)
            .ok_or_else(|| TaskError::NotFound(id.to_owned()))?;

        if !t.state.can_transition_to(TaskState::Running) {
            return Err(TaskError::BadTransition {
                id: id.to_owned(),
                from: t.state.as_str(),
                to: TaskState::Running.as_str(),
            });
        }

        *t.paused.lock().unwrap_or_else(|e| e.into_inner()) = false;
        t.state = TaskState::Running;
        t.throttle.reset();
        t.resume.notify_waiters();
        drop(live);
        self.persist_state(id, TaskState::Running, None);
        Ok(())
    }

    /// Список задач из базы — включая завершённые и оставшиеся от прошлых запусков.
    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        Ok(store::list(&self.db)?)
    }

    pub fn get(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(store::get(&self.db, id)?)
    }

    /// Занять место в полосе и стать выполняющейся — атомарно, под одним замком.
    ///
    /// Проверка места и смена состояния нарочно неразделимы: две задачи, проснувшиеся
    /// одновременно, иначе обе увидели бы одно свободное место — и обе бы стартовали,
    /// две подготовки в полосе на одну.
    fn try_claim_lane(&self, id: &str) -> ClaimOutcome {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = live.get(id) else {
            return ClaimOutcome::Gone;
        };
        let lane = t.kind.lane();
        // Себя в счёт не берём. Задача, поднятая после перезапуска и продолженная
        // человеком, уже числится выполняющейся — и, считая себя, никогда бы
        // не дождалась свободного места в собственной полосе.
        let used = live
            .iter()
            .filter(|(other_id, x)| {
                other_id.as_str() != id && x.state.occupies_lane() && x.kind.lane() == lane
            })
            .count();
        if used >= self.limits.for_lane(lane) {
            return ClaimOutcome::Busy;
        }

        // Очередь соблюдается: место занимает та задача, что стоит в полосе первой.
        // Без этой проверки порядок был бы «кто первым захватил блокировку», и
        // перестановка (FR-083) не давала бы ничего — переставлять было бы нечего.
        //
        // Сравнение только со стоящими в очереди. Приостановленная не ждёт полосы,
        // она ждёт человека, и держать за собой очередь ей не за что; продолженная
        // человеком идёт сразу — он только что сказал, что хочет именно её.
        if t.state == TaskState::Queued {
            let position = t.position;
            let есть_раньше = live.values().any(|x| {
                x.state == TaskState::Queued && x.kind.lane() == lane && x.position < position
            });
            if есть_раньше {
                return ClaimOutcome::Busy;
            }
        }

        let Some(t) = live.get_mut(id) else {
            return ClaimOutcome::Gone;
        };
        if !t.state.can_transition_to(TaskState::Running) {
            tracing::warn!(
                id,
                from = t.state.as_str(),
                "задача не может стать выполняющейся"
            );
            return ClaimOutcome::Gone;
        }
        t.state = TaskState::Running;
        drop(live);
        self.persist_state(id, TaskState::Running, None);
        ClaimOutcome::Started
    }

    fn finish(&self, id: &str, state: TaskState, error: Option<String>) {
        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = live.get_mut(id) {
                t.state = state;
            }
            live.remove(id);
        }
        self.persist_state(id, state, error.clone());
        let _ = self.events.send(TaskEvent::Done {
            id: id.to_owned(),
            state,
            error,
        });
    }

    fn persist_state(&self, id: &str, state: TaskState, error: Option<String>) {
        // Ошибка проходит вырезание секретов: она может прийти от чужой
        // библиотеки, которая о наших правилах не знает (принцип IV).
        let error = error.map(|e| crate::store::redact::redact(&e).into_owned());
        match store::save_state(&self.db, id, state, error.as_deref()) {
            Ok(true) => {}
            Ok(false) => tracing::warn!(id, "состояние сохранять некуда: записи о задаче нет"),
            Err(e) => tracing::error!(id, error = %e, "не удалось сохранить состояние задачи"),
        }
    }
}
