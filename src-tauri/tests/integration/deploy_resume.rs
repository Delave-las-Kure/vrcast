//! T299 — развёртывание, прерванное на разных шагах, доводится повторным запуском.
//!
//! **На разных, а не на одном.** SC-015 обещает сто процентов случаев, а прерывание в одном
//! месте проверяет одно место. Здесь два, и выбраны они не по вкусу:
//!
//! - **после настроек** — половина работы сделана, служба ещё не тронута. Обычная середина;
//! - **после закалки** — самый неприятный случай из всех. Вход по паролю уже выключен, а
//!   развёртывание ещё не закончено, и продолжать надо ключом, которого в профиле может не
//!   быть. Прерваться здесь и не суметь продолжить — это запертый владелец.
//!
//! Прерывание сделано отменой, а не убийством: движок спрашивает «отменили?» перед каждым
//! шагом, и это ровно тот путь, которым отмена приходит от человека.

use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, machine, Context, DeployError, Proofs};
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

/// Сколько шагов пропустить, прежде чем отменять.
fn after(n: usize) -> impl Fn() -> bool + Sync {
    let seen = AtomicUsize::new(0);
    move || seen.fetch_add(1, Ordering::SeqCst) >= n
}

#[tokio::test]
async fn an_interrupted_deployment_is_finished_by_running_it_again() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the resume check").expect("no key was made");
    let conn = by_password(&target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");

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
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect();

    // --- прерывание посередине ---
    //
    // Отменяем, когда позади уже есть пакеты и настройки. Точное число шагов здесь не
    // важно, важно, что часть работы сделана и записана на сервере.
    let stop_midway = after(5);
    match deploy::run(&ctx, &steps, &stop_midway, &mut |_| {}).await {
        Err(DeployError::Cancelled) => {}
        other => panic!("the run was not interrupted where it was told to be: {other:?}"),
    }

    // Сервер теперь наполовину настроен — и **узнаётся как наш незаконченный**, а не как
    // чужой. Без этого различения повторный запуск был бы невозможен вовсе.
    let said = target
        .exec_inside(&vrcast_studio_lib::server::detect::command(VIDEO_DIR))
        .expect("the machine would not answer");
    let half = vrcast_studio_lib::domain::server_state::judge(
        &vrcast_studio_lib::server::detect::read(&said),
    );
    assert_eq!(
        half.kind,
        vrcast_studio_lib::domain::server_state::Kind::Unfinished,
        "a half-finished deployment of ours was read as {:?}",
        half.kind
    );

    // --- прерывание после закалки ---
    //
    // Второй заход отменяем позже: к этому моменту вход по паролю уже выключен. Соединение
    // у нас открыто и переживёт это, но всякое новое пойдёт ключом — тем самым, который
    // приложение завело само.
    let never = || false;
    let stop_late = after(9);
    let outcome = deploy::run(&ctx, &steps, &stop_late, &mut |_| {}).await;
    assert!(
        matches!(outcome, Err(DeployError::Cancelled)),
        "the second run was not interrupted: {outcome:?}"
    );
    assert!(
        password_refused(&target).await,
        "the hardening step did not take, so this check is not about what it says it is"
    );

    // --- и доводим ---
    let done = deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the deployment could not be finished after two interruptions");

    for step in &done {
        assert!(
            !matches!(step.status, Status::NotApplied | Status::Failed { .. }),
            "{:?} came back as {:?} after the run that was supposed to finish everything",
            step.id,
            step.status
        );
    }

    // И то, что было сделано до прерываний, не переделывалось: этот прогон видел их
    // сделанными. Спрошено у сервера, а не у нашего отчёта.
    let seen = target
        .exec_inside(
            "id vrcast >/dev/null 2>&1 && echo user
test -f /etc/vrcast/state.json && echo state
[ \"$(systemctl is-active caddy)\" = active ] && echo serving
sshd -T | grep -qx 'passwordauthentication no' && echo password-off",
        )
        .expect("the machine would not answer");
    for expected in ["user", "state", "serving", "password-off"] {
        assert!(
            seen.contains(expected),
            "{expected} is missing after the deployment was finished: {seen}"
        );
    }
}
