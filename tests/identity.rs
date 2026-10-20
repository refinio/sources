use sources::git::normalized_identity;

#[test]
fn standard_https_ssh_and_scp_spellings_identify_the_same_repository() {
    for url in [
        "https://GitHub.com/Owner/Repo.git",
        "ssh://git@github.com/Owner/Repo",
        "git@github.com:Owner/Repo.git",
    ] {
        assert_eq!(normalized_identity(url).unwrap(), "github.com/Owner/Repo");
    }
}

#[test]
fn repository_case_and_nondefault_ports_are_not_silently_equated() {
    let standard = normalized_identity("https://example.com/Owner/Repo").unwrap();
    assert_ne!(
        standard,
        normalized_identity("https://example.com/owner/repo").unwrap()
    );
    assert_ne!(
        standard,
        normalized_identity("ssh://git@example.com:443/Owner/Repo").unwrap()
    );
    assert_eq!(
        standard,
        normalized_identity("ssh://git@example.com:22/Owner/Repo.git").unwrap()
    );
    assert_ne!(
        normalized_identity("https://example.com:8443/Owner/Repo").unwrap(),
        normalized_identity("ssh://git@example.com:8443/Owner/Repo").unwrap()
    );
}

#[test]
fn ssh_hosts_are_case_insensitive_and_credentials_do_not_identify_a_repository() {
    assert_eq!(
        normalized_identity("ssh://alice:secret@GitHub.COM/Owner/Repo.git").unwrap(),
        "github.com/Owner/Repo"
    );
    assert_eq!(
        normalized_identity("alice@GitHub.COM:Owner/Repo.git").unwrap(),
        "github.com/Owner/Repo"
    );
}
