//! T018 — запуск внешних программ так, чтобы их можно было гарантированно убить.
//!
//! Конституция, принцип III (НЕОБСУЖДАЕМО). Это не предосторожность на всякий случай, а
//! записанный урок проекта: осиротевший `ffmpeg` продолжает писать в тот же файл и портит
//! его молча — пользователь узнаёт об этом от зрителей, когда чинить уже поздно.
//!
//! Обычного `kill` по идентификатору процесса недостаточно: `ffmpeg` и `ssh` порождают
//! собственных потомков, и убийство родителя оставляет их работать. Поэтому каждый запуск
//! получает **собственную группу**, и завершается группа целиком:
//!
//! | ОС | Механизм | Что даёт сверх обычного завершения |
//! |---|---|---|
//! | Windows | объект задания с завершением по закрытию | потомки умирают, **даже если приложение убито диспетчером задач**: описатель закрывает ядро |
//! | Unix | отдельная группа процессов, сигнал всей группе | сигнал доходит до всех потомков, а не только до прямого |
//!
//! Строка про Windows — ключевая. Она закрывает сценарий, в котором никакой код приложения
//! уже не выполняется, а гарантия всё равно обязана держаться (SC-010).

use std::process::Stdio;
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("не удалось запустить {program}: {reason}")]
    Spawn { program: String, reason: String },

    #[error("не удалось завершить дерево процессов: {0}")]
    Kill(String),

    #[error("приостановка процесса не удалась: {0}")]
    Suspend(String),
}

pub type Result<T> = std::result::Result<T, ProcessError>;

/// Внешняя программа, запущенная в собственной группе.
///
/// Пока эта структура жива, дерево процессов удерживается; `kill_tree` завершает его целиком.
pub struct ManagedProcess {
    child: Child,
    program: String,
    #[cfg(windows)]
    job: windows_job::Job,
}

impl ManagedProcess {
    /// Запустить программу в собственной группе процессов.
    ///
    /// **Не вызывать из недолговечных потоков.** На Linux защита опирается на сигнал,
    /// который ядро шлёт при смерти родительского *потока*, а не процесса. Если породить
    /// ffmpeg из потока, который вскоре завершится, ffmpeg умрёт вместе с ним посреди
    /// работы. Рабочие потоки исполнителя задач живут всё время работы приложения —
    /// оттуда вызывать можно; из отдельных потоков для блокирующих операций — нельзя.
    pub fn spawn(program: &str, args: &[String]) -> Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        {
            // Типаж `std::os::unix::process::CommandExt` здесь НЕ нужен: `pre_exec`
            // у `tokio::process::Command` собственный. Лишний импорт компилируется
            // только под Unix — под Windows этот кусок не собирается вовсе, — и
            // поймал его лишь прогон в непрерывной интеграции под Linux.
            //
            // Идентификатор родителя запоминаем ДО порождения: он понадобится потомку,
            // чтобы закрыть щель, описанную ниже.
            let parent_pid = std::process::id();
            unsafe {
                cmd.pre_exec(move || {
                    // Своя группа процессов: сигнал уйдёт всем потомкам, а не только прямому.
                    if libc_setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }

                    #[cfg(target_os = "linux")]
                    {
                        // Ядро само убьёт потомка, когда умрёт родитель, — и сделает это,
                        // даже если никакой наш код уже не выполняется: приложение убито
                        // сигналом, съедено убийцей при нехватке памяти или упало в панике.
                        // Это единственный на Linux аналог того, что на Windows даёт
                        // объект задания.
                        //
                        // Два ограничения, о которых надо помнить:
                        //   1. Действует только на ПРЯМОГО потомка. Внуков закрывает
                        //      уборка при запуске (см. tasks::registry).
                        //   2. Срабатывает при смерти родительского ПОТОКА, а не процесса.
                        //      Поэтому порождать процессы из недолговечных потоков нельзя —
                        //      см. предупреждение в описании ManagedProcess::spawn.
                        const PR_SET_PDEATHSIG: i32 = 1;
                        const SIGKILL: i64 = 9;
                        libc_prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);

                        // Щель: родитель мог умереть между порождением и строкой выше —
                        // тогда сигнал уже не придёт никогда. Сверяем и уходим сами.
                        //
                        // Уходить обязательно через _exit: между fork и exec разрешены
                        // только async-signal-safe вызовы, а обычный exit запускает
                        // atexit-обработчики и сброс потоков вывода — они могут навсегда
                        // повиснуть на замке, оставленном потоком родителя, и потомок
                        // станет вечным сиротой — ровно тем, от чего здесь защита.
                        if libc_getppid() != parent_pid as i32 {
                            libc_exit_now(0);
                        }
                    }

                    Ok(())
                });
            }
        }

        #[cfg(windows)]
        {
            // CREATE_SUSPENDED не используем: процесс должен попасть в задание до начала
            // работы, но приостановка усложнила бы чтение потоков. Достаточно того, что
            // назначение в задание происходит сразу после запуска, до первого чтения.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        }

        let child = cmd.spawn().map_err(|e| ProcessError::Spawn {
            program: program.to_owned(),
            reason: e.to_string(),
        })?;

        #[cfg(windows)]
        let job = {
            let job = windows_job::Job::create().map_err(|e| ProcessError::Spawn {
                program: program.to_owned(),
                reason: format!("не удалось создать объект задания: {e}"),
            })?;
            if let Some(handle) = child.raw_handle() {
                job.assign(handle).map_err(|e| ProcessError::Spawn {
                    program: program.to_owned(),
                    reason: format!("не удалось назначить процесс заданию: {e}"),
                })?;
            }
            job
        };

        tracing::debug!(program, pid = ?child.id(), "внешняя программа запущена в своей группе");

        Ok(Self {
            child,
            program: program.to_owned(),
            #[cfg(windows)]
            job,
        })
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// Забрать потоки вывода для чтения прогресса.
    pub fn take_output(
        &mut self,
    ) -> (
        Option<tokio::process::ChildStdout>,
        Option<tokio::process::ChildStderr>,
    ) {
        (self.child.stdout.take(), self.child.stderr.take())
    }

    /// Дождаться завершения.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Завершить всё дерево процессов.
    ///
    /// Жёстко и сразу: результат прерванной задачи всё равно отбрасывается, а ожидание
    /// вежливого выхода — та самая щель, в которую пролезает осиротевший процесс.
    pub async fn kill_tree(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            // Завершает ВСЕ процессы задания, включая внуков.
            self.job
                .terminate()
                .map_err(|e| ProcessError::Kill(e.to_string()))?;
        }

        #[cfg(unix)]
        {
            if let Some(pid) = self.child.id() {
                // Отрицательный идентификатор = вся группа процессов.
                unsafe {
                    libc_killpg(pid as i32, 9);
                }
            }
        }

        // Прямого потомка добиваем в любом случае и дожидаемся, чтобы не оставить зомби.
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;

        tracing::debug!(program = %self.program, "дерево процессов завершено");
        Ok(())
    }

    /// Приостановить выполнение, не теряя проделанной работы (FR-083a).
    pub fn suspend(&self) -> Result<()> {
        self.signal_stop(true)
    }

    /// Продолжить приостановленное выполнение.
    pub fn resume(&self) -> Result<()> {
        self.signal_stop(false)
    }

    #[cfg(unix)]
    fn signal_stop(&self, stop: bool) -> Result<()> {
        let Some(pid) = self.child.id() else {
            return Err(ProcessError::Suspend(String::from("процесс уже завершён")));
        };
        // SIGSTOP / SIGCONT всей группе: приостанавливать надо и потомков.
        let sig = if stop { 19 } else { 18 };
        unsafe {
            libc_killpg(pid as i32, sig);
        }
        Ok(())
    }

    #[cfg(windows)]
    fn signal_stop(&self, stop: bool) -> Result<()> {
        let Some(pid) = self.child.id() else {
            return Err(ProcessError::Suspend(String::from("процесс уже завершён")));
        };
        windows_job::suspend_process(pid, stop).map_err(ProcessError::Suspend)
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "setsid"]
    fn libc_setsid() -> i32;
    #[link_name = "killpg"]
    fn libc_killpg(pgrp: i32, sig: i32) -> i32;
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
    #[cfg(target_os = "linux")]
    #[link_name = "getppid"]
    fn libc_getppid() -> i32;
    #[cfg(target_os = "linux")]
    #[link_name = "prctl"]
    fn libc_prctl(option: i32, a2: i64, a3: i64, a4: i64, a5: i64) -> i32;
    /// Немедленный выход без atexit-обработчиков — единственный безопасный
    /// способ завершиться между fork и exec.
    #[cfg(target_os = "linux")]
    #[link_name = "_exit"]
    fn libc_exit_now(status: i32) -> !;
}

/// Завершить процесс по идентификатору — для уборки уцелевших при запуске.
///
/// Отдельно от [`ManagedProcess::kill_tree`]: там процесс наш и живой, здесь — чужой
/// по отношению к текущему запуску приложения, оставшийся от предыдущего.
pub(crate) fn kill_pid(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc_kill(pid as i32, 9) == 0 }
    }
    #[cfg(windows)]
    {
        windows_job::terminate_pid(pid)
    }
}

/// Имя выполняемого файла процесса, если он ещё жив.
///
/// Нужно не для красоты отчёта, а ради безопасности уборки: идентификаторы процессов
/// переиспользуются, и за старым номером к моменту следующего запуска может стоять
/// совершенно посторонняя программа. Убивать её недопустимо.
pub(crate) fn process_name(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_owned())
    }
    #[cfg(windows)]
    {
        windows_job::process_name(pid)
    }
}

/// Опознавательный признак процесса — время его запуска.
///
/// Нужен уборке при запуске: перед тем как добить уцелевшую программу, надо
/// убедиться, что за её номером не оказалась посторонняя. Номера переиспользуются,
/// и убить чужое дороже, чем не добить своё.
///
/// **Почему не имя.** Имя лжёт, и это выяснилось на живом примере: `sh -c "sleep"`
/// заменяет себя на `sleep`, и записанное имя перестаёт совпадать. Вдобавок
/// `/proc/<pid>/comm` обрезан пятнадцатью знаками, так что программа с длинным
/// именем не совпадёт сама с собой. Время запуска не меняется при подмене образа,
/// не обрезается и различает два процесса с одним номером точно.
///
/// `None` означает «узнать не удалось» — тогда остаётся сверка по имени, которая
/// хуже, но лучше, чем ничего.
pub(crate) fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        // В /proc/<pid>/stat двадцать второе поле — время запуска в тактах от
        // загрузки системы. Разбор осложняет второе поле: имя программы в скобках,
        // и в нём бывают и пробелы, и сами скобки. Поэтому отсчитываем от ПОСЛЕДНЕЙ
        // закрывающей скобки, а не разбиваем строку по пробелам с начала.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_name = &stat[stat.rfind(')')? + 1..];
        let field = after_name.split_whitespace().nth(19)?;
        Some(field.to_owned())
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // На прочих Unix надёжного способа без лишних зависимостей нет.
        let _ = pid;
        None
    }
    #[cfg(windows)]
    {
        windows_job::process_created_at(pid)
    }
}

#[cfg(windows)]
mod windows_job {
    //! Минимальная обвязка над объектом задания Windows.
    //!
    //! Отдельной зависимости не берём: нужны четыре вызова, и каждый лишний пакет
    //! в дереве — это ещё одна лицензия к проверке (приложение под GPL).

    use std::ffi::c_void;

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic: BasicLimitInformation,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: u32, info: *mut c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(h: Handle) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> Handle;
        fn SuspendThread(thread: Handle) -> u32;
        fn ResumeThread(thread: Handle) -> u32;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    /// Время в системном виде: два тридцатидвухбитных слова.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    pub struct Job(Handle);

    // Описатель задания принадлежит одному владельцу и передаётся между потоками
    // вместе с ним.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn create() -> Result<Self, String> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }

                // Вот ради этой строки всё и затевалось: когда описатель задания
                // закрывается — а ядро закрывает его при смерти процесса-владельца,
                // включая убийство через диспетчер задач, — все процессы задания
                // завершаются вместе с ним.
                let mut info = ExtendedLimitInformation {
                    basic: BasicLimitInformation {
                        limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let ok = SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    &mut info as *mut _ as *mut c_void,
                    std::mem::size_of::<ExtendedLimitInformation>() as u32,
                );
                if ok == 0 {
                    let e = std::io::Error::last_os_error().to_string();
                    CloseHandle(job);
                    return Err(e);
                }
                Ok(Self(job))
            }
        }

        pub fn assign(&self, process: Handle) -> Result<(), String> {
            unsafe {
                if AssignProcessToJobObject(self.0, process) == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            Ok(())
        }

        pub fn terminate(&self) -> Result<(), String> {
            unsafe {
                if TerminateJobObject(self.0, 1) == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Завершить процесс по идентификатору. Используется уборкой уцелевших при запуске.
    pub fn terminate_pid(pid: u32) -> bool {
        const PROCESS_TERMINATE: u32 = 0x0001;
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            ok
        }
    }

    /// Имя выполняемого файла живого процесса — для сверки перед убийством.
    /// Время запуска процесса — точный опознавательный признак.
    ///
    /// Номера процессов переиспользуются, а время запуска у нового процесса
    /// с тем же номером будет другим. Этого достаточно, чтобы не убить чужое.
    pub fn process_created_at(pid: u32) -> Option<String> {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut creation = FileTime::default();
            let mut ignored = FileTime::default();
            let ok = GetProcessTimes(h, &mut creation, &mut ignored, &mut ignored, &mut ignored);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            Some(format!(
                "{}",
                (u64::from(creation.high) << 32) | u64::from(creation.low)
            ))
        }
    }

    pub fn process_name(pid: u32) -> Option<String> {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            // Разбор пути штатными средствами: они знают про разделители обеих ОС.
            std::path::Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .or(Some(full))
        }
    }

    /// Приостановить или продолжить процесс, перебрав его потоки.
    ///
    /// В Windows нет средства приостановить процесс целиком одним вызовом — только
    /// поток за потоком.
    pub fn suspend_process(pid: u32, stop: bool) -> Result<(), String> {
        const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot as isize == -1 {
                return Err(std::io::Error::last_os_error().to_string());
            }

            let mut entry = ThreadEntry32 {
                size: std::mem::size_of::<ThreadEntry32>() as u32,
                ..Default::default()
            };

            let mut touched = 0;
            let mut ok = Thread32First(snapshot, &mut entry);
            while ok != 0 {
                if entry.owner_process_id == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id);
                    if !thread.is_null() {
                        if stop {
                            SuspendThread(thread);
                        } else {
                            ResumeThread(thread);
                        }
                        CloseHandle(thread);
                        touched += 1;
                    }
                }
                entry.size = std::mem::size_of::<ThreadEntry32>() as u32;
                ok = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);

            if touched == 0 {
                return Err(String::from("у процесса не найдено ни одного потока"));
            }
        }
        Ok(())
    }
}
