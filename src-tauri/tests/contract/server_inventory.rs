//! T337 — состав серверной части версии 1 заморожен, и это проверяется.
//!
//! **Заморожен — значит не может разъехаться молча.** `versions.json` — это то, что
//! приложение обещает про развёрнутый сервер: из чего он состоит, какими шагами собран и
//! какие файлы ему принадлежат. Обещание, которое никто не сверяет с кодом, живёт ровно до
//! первого изменения кода — а дальше выглядит выполненным, не будучи им.
//!
//! Отсюда правило R-11: версия целая и растёт на единицу при **любом** изменении состава.
//! Значит и обратное: пока версия единица, состав обязан быть тем самым. Проверки ниже
//! сверяют опись с четырьмя источниками сразу — списком шагов, списком пакетов, папкой
//! ресурсов и постоянной, которую приложение пишет в сам сервер.
//!
//! Каждая из них — сторож против одной и той же осечки: изменить состав и не заметить, что
//! у уже развёрнутых серверов он остался прежним, а версия по-прежнему обещает единицу.

use std::collections::BTreeSet;

use serde_json::Value;
use vrcast_studio_lib::domain::deploy_steps::ORDER;
use vrcast_studio_lib::domain::server_state::APP_EXPECTS;
use vrcast_studio_lib::server::deploy::packages::FROM_APT;
use vrcast_studio_lib::server::deploy::{fail2ban, updates};

const INVENTORY: &str = include_str!("../../resources/server/versions.json");

fn inventory() -> Value {
    serde_json::from_str(INVENTORY).expect("versions.json is not readable JSON")
}

#[test]
fn the_declared_version_is_the_one_the_application_writes() {
    let it = inventory();
    let declared = it["vrcast_server_version"]
        .as_u64()
        .expect("the composition declares no version");
    assert_eq!(
        declared as u32, APP_EXPECTS,
        "the inventory says the server side is version {declared} while the application \
         deploys and expects {APP_EXPECTS}. One of the two is lying to whoever reads it."
    );
}

#[test]
fn the_steps_of_the_inventory_are_the_steps_that_run() {
    // В том же порядке, а не просто тем же составом: порядок здесь — это и есть половина
    // состава (R-12), и опись, перечисляющая шаги в другом порядке, описывает другой сервер.
    let it = inventory();
    let declared: Vec<String> = it["steps"]
        .as_array()
        .expect("the composition lists no steps")
        .iter()
        .map(|s| s.as_str().expect("a step is not a string").to_owned())
        .collect();
    let actual: Vec<String> = ORDER
        .iter()
        .map(|s| s.in_the_inventory().to_owned())
        .collect();

    assert_eq!(
        declared, actual,
        "the inventory's steps and the deployment's have diverged. Change one and the version \
         has to go up with it (R-11)."
    );
}

#[test]
fn the_packages_of_the_inventory_are_the_packages_installed() {
    let it = inventory();
    let declared: BTreeSet<String> = it["components"]
        .as_array()
        .expect("the composition lists no components")
        .iter()
        .map(|c| c["id"].as_str().expect("a component has no id").to_owned())
        .collect();

    // Caddy ставится своим путём — из собственного репозитория, а не одной строкой с
    // остальными, — но в описи стоит наравне с ними: человеку важно, что оно на его машине,
    // а не как оно туда попало. Так же и с двумя пакетами, которые ставят себе собственные
    // шаги: имена берутся из самих шагов, а не переписаны сюда.
    let mut actual: BTreeSet<String> = FROM_APT.iter().map(|s| String::from(*s)).collect();
    actual.insert(String::from("caddy"));
    actual.insert(String::from(fail2ban::PACKAGE));
    actual.insert(String::from(updates::PACKAGE));

    // Не равенство: опись перечисляет то, что **составляет** серверную часть, а `curl`,
    // `tar`, `gnupg` и `ca-certificates` — это то, чем её ставят. Обязательно другое: всё,
    // что названо в описи, должно ставиться на самом деле.
    for named in &declared {
        assert!(
            actual.contains(named),
            "the inventory promises {named} on the server and nothing installs it"
        );
    }
    // И наоборот — то, что осталось работать на машине, обязано быть названо.
    for stays in ["ffmpeg", "ufw", "caddy"] {
        assert!(
            declared.contains(stays),
            "{stays} is installed on somebody's server and the inventory does not mention it"
        );
    }
}

#[test]
fn every_file_the_inventory_claims_has_a_resource_behind_it() {
    // Опись обещает, что такие-то файлы на сервере принадлежат приложению. Обещание про файл,
    // содержимого для которого нет, — это шаг, который однажды напишет туда пустоту.
    let it = inventory();
    let files = it["files"]
        .as_object()
        .expect("the composition lists no files");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/server");

    for (path, source) in files {
        let source = source.as_str().expect("a file's source is not a string");
        // Значение — либо имя файла в ресурсах, либо объяснение, почему его там нет.
        // Объяснение отличается от имени тем, что в нём есть пробелы: имён с пробелами
        // среди ресурсов нет и не будет.
        if source.contains(' ') {
            continue;
        }
        assert!(
            dir.join(source).exists(),
            "the inventory promises {path} on the server, and its content {source} is not in \
             resources/server"
        );
    }
}

#[test]
fn what_the_owner_removed_stays_removed() {
    // Решения 2026-08-27 (T252, T323). Оба вычёркивали, а вычеркнутое возвращается тихо: его
    // добавляет тот, кто не читал, почему его нет. Возвращать можно — но новым решением и с
    // новой версией серверной части, а не мимоходом.
    let text = INVENTORY.to_lowercase();
    for gone in ["mediamtx", "rive"] {
        let mentioned_outside_the_note = inventory()["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"].as_str().is_some_and(|id| id.contains(gone)));
        assert!(
            !mentioned_outside_the_note,
            "{gone} is back in the composition. It was taken out by a decision; putting it \
             back needs another one, and a new version of the server side with it (R-11)."
        );
    }
    // И объяснение, почему его нет, остаётся в файле: без него следующий читатель увидит
    // просто отсутствие и решит, что про это забыли.
    assert!(
        text.contains("mediamtx"),
        "the note saying why MediaMTX is absent has gone, and absence without a reason reads \
         as an oversight"
    );
}
