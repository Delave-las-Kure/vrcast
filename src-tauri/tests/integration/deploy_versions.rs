//! T301, T302 — версии серверной части и доказанная закалка.
//!
//! Две проверки, и обе про то, что нельзя установить чтением файлов.
//!
//! **Версия.** Развёртывания здесь нет вовсе: файл состояния кладётся руками. Определитель
//! читает именно его, и разворачивать целый сервер ради одной строки в JSON значило бы
//! тратить полторы минуты на то, что проверяется за секунду.
//!
//! **Закалка.** А вот её проверить чтением нельзя в принципе — в этом весь смысл. На боевом
//! сервере шаг был написан, отработал без жалоб, и полгода `sshd -T` отвечал, что вход по
//! паролю разрешён. Здесь спрашивается сам сервер, настоящей попыткой войти.

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::server_state::{self, Compat, Kind, APP_EXPECTS};
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs};
use vrcast_studio_lib::server::detect;
use vrcast_studio_lib::server::gate::{allowed, Intent};
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

/// Что говорит определитель про машину с таким файлом состояния.
fn state_with(target: &DeployTarget, version: u32) -> server_state::ServerState {
    target
        .exec_inside(&format!(
            "mkdir -p /etc/vrcast && printf '%s\\n' '{{\"vrcast_server_version\": {version}}}' > /etc/vrcast/state.json"
        ))
        .expect("could not write a state file");
    let said = target
        .exec_inside(&detect::command(VIDEO_DIR))
        .expect("the machine would not answer");
    server_state::judge(&detect::read(&said))
}

#[tokio::test]
async fn a_newer_server_side_is_read_and_never_written_to() {
    // FR-130. Опасно здесь обратное прочтение: у новой серверной части вещи могут лежать
    // иначе, и приложение постарше, записывающее их по своей раскладке, — это способ тихо
    // сломать работающий сервер.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");

    let newer = state_with(&target, APP_EXPECTS + 1);
    assert_eq!(newer.kind, Kind::Managed);
    assert_eq!(newer.compat, Compat::TooNew);
    assert_eq!(newer.server_version, Some(APP_EXPECTS + 1));

    let may = allowed(&newer, Intent::Change);
    assert!(may.is_err(), "a newer server side was opened for writing");
    assert!(
        allowed(&newer, Intent::Read).is_ok(),
        "and it must still be possible to look at it"
    );
    assert!(
        allowed(&newer, Intent::Setup).is_err(),
        "an upgrade was offered to a version this application does not know"
    );
}

#[tokio::test]
async fn our_own_version_is_read_from_the_file_and_not_assumed() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");

    let ours = state_with(&target, APP_EXPECTS);
    assert_eq!(ours.kind, Kind::Managed);
    assert_eq!(ours.compat, Compat::Ok);
    assert!(allowed(&ours, Intent::Change).is_ok());

    // А испорченный файл — это не отсутствующий файл. Считать его отсутствующим значит
    // развернуть поверх собственного сервера, заменив каталог раздачи и домен тем, что
    // скажут новому развёртыванию.
    target
        .exec_inside("printf '%s' '{\"vrcast_server_ver' > /etc/vrcast/state.json")
        .expect("could not spoil the state file");
    let said = target
        .exec_inside(&detect::command(VIDEO_DIR))
        .expect("the machine would not answer");
    let spoilt = server_state::judge(&detect::read(&said));
    assert_eq!(
        spoilt.kind,
        Kind::Foreign,
        "a marker we cannot read left the machine looking like ours"
    );
    assert!(allowed(&spoilt, Intent::Change).is_err());
}

#[tokio::test]
async fn the_password_is_refused_by_the_server_itself_after_the_hardening() {
    // **Не «в файле написано `no`».** Строка в файле доказывает, что строка есть в файле —
    // и это ровно то, чем шесть месяцев прикрывалась незакрытая дверь на боевом сервере.
    //
    // Разворачивается только то, что для этого нужно: ключ и закалка. Пакеты, каталоги и
    // раздача к вопросу отношения не имеют, а полторы минуты стоят.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the hardening check").expect("no key was made");
    let conn = by_password(&target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");

    // Пока — пускает. Иначе проверять нечего: шаг обязан что-то **изменить**.
    assert!(
        !password_refused(&target).await,
        "a freshly made server already refuses passwords, so this check proves nothing"
    );

    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(&target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
    let ctx = Context {
        conn: &conn,
        domain: "vrcast-container.invalid",
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours: false,
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| matches!(s.id, StepId::SshKey | StepId::SshHardening))
        .collect();

    let never = || false;
    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the key and the hardening would not go in");

    // Сервер спрошен сам, новым соединением. То, что мы держим, продолжило бы работать что
    // бы мы ни сделали с настройками, — и именно поэтому оно плохой свидетель.
    assert!(
        password_refused(&target).await,
        "the password still gets in after the hardening step said it was done"
    );
    assert!(
        key_works(&target, &made.private_openssh).await,
        "the password is off and the key does not work — this is a locked-out owner"
    );
}
