use std::collections::BTreeMap;

use scippy_loci::facts::{Change, NotesIndex, Policy, Profile, SourceFacts, Status};
use scippy_loci::{Settings, analyze_note};
use sha2::{Digest, Sha256};

fn profile() -> Profile {
    Profile {
        format_version: 1,
        extractor_version: 1,
        pack_digests: ["markdown", "markdown_inline", "yaml"]
            .map(|n| (n.into(), format!("sha256:{}", "0".repeat(64))))
            .into(),
    }
}

fn source(path: &str, text: &str) -> SourceFacts {
    SourceFacts {
        path: path.into(),
        text: Some(text.into()),
        sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        note: Some(analyze_note(text.as_bytes(), &Settings::default())),
        status: Status::Indexed,
    }
}

fn fresh(
    sources: &BTreeMap<String, SourceFacts>,
    attachments: Vec<String>,
    policy: Policy,
) -> NotesIndex {
    NotesIndex::new(
        profile(),
        policy,
        sources.values().cloned().collect(),
        attachments,
    )
    .unwrap()
}

#[test]
fn incremental_matches_clean_after_names_anchors_and_source_changes() {
    let mut sources = BTreeMap::from([
        (
            "A.md".into(),
            source(
                "A.md",
                "[[Late]] [[Dup]] [[Nick]] [[B#Plans]] [[B#^p]] [[pic.png]]\n",
            ),
        ),
        (
            "B.md".into(),
            source("B.md", "---\naliases: [Nick]\n---\n# Plans\n\ntext ^p\n"),
        ),
        ("one/Dup.md".into(), source("one/Dup.md", "# One\n")),
    ]);
    let attachments = vec!["pic.png".into()];
    let mut index = fresh(&sources, attachments.clone(), Policy::default());
    let mut seed = 42u64;
    for step in 0..600 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let n = (seed >> 32) % 8;
        let path = match step % 6 {
            0 => "Late.md".into(),
            1 => format!("folder{n}/Dup.md"),
            2 => "Nick.md".into(),
            _ => "B.md".into(),
        };
        let next = match step % 6 {
            0..=2 if n % 2 == 0 => None,
            0..=2 => Some(source(&path, "# Added\n")),
            3 => Some(source(
                &path,
                &format!("---\naliases: [Nick, 'Nick {n}']\n---\n# Plans\n\n😀 text ^p\r\n"),
            )),
            4 => Some(source(
                &path,
                &format!("prefix {step}\n\n# Schedule\n\nfirst ^p\n\nsecond ^p\n[[Late]]\n"),
            )),
            _ => None,
        };
        let change = Change {
            path: path.clone(),
            base_sha256: sources.get(&path).map(|s| s.sha256.clone()),
            source: next.clone(),
        };
        index
            .update(index.generation(), &profile(), vec![change], None)
            .unwrap();
        match next {
            Some(s) => {
                sources.insert(path, s);
            }
            None => {
                sources.remove(&path);
            }
        }
        assert_eq!(
            index.records(),
            fresh(&sources, attachments.clone(), Policy::default()).records(),
            "step {step}"
        );
    }
}

#[test]
fn failed_batch_and_profile_changes_leave_the_index_unchanged() {
    let a = source("A.md", "[[B]]");
    let mut index = NotesIndex::new(profile(), Policy::default(), vec![a.clone()], vec![]).unwrap();
    let before = index.records().clone();
    let changes = vec![
        Change {
            path: "A.md".into(),
            base_sha256: Some(a.sha256),
            source: None,
        },
        Change {
            path: "B.md".into(),
            base_sha256: Some("wrong".into()),
            source: Some(source("B.md", "# B")),
        },
    ];
    assert!(index.update(1, &profile(), changes, None).is_err());
    assert_eq!(index.records(), &before);
    assert_eq!(index.generation(), 1);
    let mut changed = profile();
    changed.extractor_version = 2;
    assert!(index.update(1, &changed, vec![], None).is_err());
    assert!(index.update(0, &profile(), vec![], None).is_err());
    assert_eq!(index.records(), &before);
}

#[test]
fn attachments_and_unavailable_sources_invalidate_old_results() {
    let a = source("A.md", "[[pic.png]] [[B#Plans]]");
    let b = source("B.md", "# Plans\n");
    let mut index =
        NotesIndex::new(profile(), Policy::default(), vec![a, b.clone()], vec![]).unwrap();
    assert_eq!(index.records()["A.md"][0].state.name(), "missing");
    index
        .update(1, &profile(), vec![], Some(vec!["pic.png".into()]))
        .unwrap();
    assert_eq!(index.records()["A.md"][0].state.name(), "resolved");
    let mut unavailable = b.clone();
    unavailable.status = Status::Unavailable;
    unavailable.note = None;
    unavailable.text = None;
    index
        .update(
            2,
            &profile(),
            vec![Change {
                path: "B.md".into(),
                base_sha256: Some(b.sha256.clone()),
                source: Some(unavailable),
            }],
            None,
        )
        .unwrap();
    assert_eq!(index.records()["A.md"][1].state.name(), "skipped-target");
    index
        .update(
            3,
            &profile(),
            vec![Change {
                path: "B.md".into(),
                base_sha256: Some(b.sha256.clone()),
                source: Some(b),
            }],
            Some(vec![]),
        )
        .unwrap();
    assert_eq!(index.records()["A.md"][0].state.name(), "missing");
    assert_eq!(index.records()["A.md"][1].anchor_state, "resolved");
}

#[test]
fn root_local_precedence_and_atomic_rename_match_clean() {
    let mut sources: BTreeMap<_, _> = [
        source("repo/A.md", "[[B#Plans]]"),
        source("repo/B.md", "# Plans"),
        source("B.md", "# Elsewhere"),
    ]
    .into_iter()
    .map(|s| (s.path.clone(), s))
    .collect();
    let policy = Policy {
        roots: vec![("repo".into(), false)],
        ..Policy::default()
    };
    let mut index = fresh(&sources, vec![], policy.clone());
    assert_eq!(
        index.records()["repo/A.md"][0].target_path.as_deref(),
        Some("repo/B.md")
    );
    let old = sources.remove("repo/B.md").unwrap();
    let next = source("repo/C.md", "# Plans");
    sources.insert(next.path.clone(), next.clone());
    index
        .update(
            1,
            &profile(),
            vec![
                Change {
                    path: old.path,
                    base_sha256: Some(old.sha256),
                    source: None,
                },
                Change {
                    path: next.path.clone(),
                    base_sha256: None,
                    source: Some(next),
                },
            ],
            None,
        )
        .unwrap();
    assert_eq!(index.records(), fresh(&sources, vec![], policy).records());
}

#[test]
fn source_admission_rejects_wrong_hash_and_split_character() {
    let mut s = source("A.md", "😀 [[B]]");
    s.note.as_mut().unwrap().relations[0].span = (1, 8);
    assert!(s.validate().is_err());
    s = source("A.md", "[[B]]");
    s.sha256 = "0".repeat(64);
    assert!(s.validate().is_err());
    s = source("../A.md", "hello");
    assert!(s.validate().is_err());
}
