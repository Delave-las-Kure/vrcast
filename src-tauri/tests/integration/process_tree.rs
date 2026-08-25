//! A check that the integration fixture cleans up after itself.
//!
//! The fixture starts containers, and if it does not remove them, dangling servers pile up
//! in the system after a failed run. That is exactly the class of fault an orphaned encoding
//! process is — one floor up.

use super::fixture::{docker_available, TestServer, IMAGE};

/// How many of OUR containers are running right now.
///
/// Counting the whole of `docker ps -q` will not do: the daemon is shared, and an unrelated
/// container started between two readings would fail the test for nothing. A failure of the
/// count itself is also a failure rather than "let it be zero".
fn our_containers() -> usize {
    let out = std::process::Command::new("docker")
        .args(["ps", "-q", "--filter", &format!("ancestor={IMAGE}")])
        .output()
        .expect("could not run docker ps");
    assert!(out.status.success(), "docker ps exited with an error");
    String::from_utf8_lossy(&out.stdout).lines().count()
}

#[test]
fn the_container_is_removed_along_with_the_test() {
    assert!(
        docker_available(),
        "Docker is not running — the integration tests cannot go ahead"
    );

    let before = our_containers();

    {
        let server = TestServer::start().expect("the container would not come up");
        assert!(
            our_containers() > before,
            "the container did not appear among those running"
        );
        drop(server);
    }

    // Docker is given a moment to clean up.
    std::thread::sleep(std::time::Duration::from_secs(2));

    assert_eq!(
        our_containers(),
        before,
        "a dangling container was left after the test"
    );
}
