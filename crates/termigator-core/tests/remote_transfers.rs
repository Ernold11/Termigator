//! `termigator_core::remote::transfers` golden corpus
//! (compat/golden/transfers_*.json) and the queue logic tests.

mod common;

use std::path::PathBuf;

use serde_json::Value as J;
use termigator_core::remote::transfers::{
    JobState, Progress, TransferJob, TransferQueue, parse_progress, split_updates,
};

fn job() -> TransferJob {
    TransferJob::new(
        true,
        PathBuf::from("/l"),
        b"/r".to_vec(),
        b"x".to_vec(),
        vec![b"true".to_vec()],
        vec![(
            "SSHPASS".to_owned(),
            termigator_core::vault::Secret::new(b"hunter2".to_vec()),
        )],
    )
}

#[test]
fn parse_matches_golden() {
    let cases = common::golden("transfers_parse");
    assert!(cases.len() >= 25);
    for case in &cases {
        let n = common::name(case);
        let got = parse_progress(case["input"].as_str().expect("input"));
        let want = match &case["expect"] {
            J::Null => None,
            e => Some(Progress {
                percent: e["percent"].as_u64().expect("percent") as u8,
                size: e["size"].as_str().expect("size").to_owned(),
                rate: e["rate"].as_str().expect("rate").to_owned(),
                eta: e["eta"].as_str().expect("eta").to_owned(),
            }),
        };
        assert_eq!(got, want, "{n}");
    }
}

#[test]
fn split_matches_golden() {
    let cases = common::golden("transfers_split");
    assert!(cases.len() >= 40);
    for case in &cases {
        let n = common::name(case);
        let mut pending = String::new();
        let steps = case["steps"].as_array().expect("steps");
        let want = case["expect"].as_array().expect("expect");
        assert_eq!(steps.len(), want.len(), "{n}");
        for (i, (chunk, w)) in steps.iter().zip(want).enumerate() {
            let (lines, tail) = split_updates(&pending, chunk.as_str().expect("chunk"));
            let want_lines: Vec<&str> = w["lines"]
                .as_array()
                .expect("lines")
                .iter()
                .filter_map(J::as_str)
                .collect();
            assert_eq!(lines, want_lines, "{n} step {i}");
            assert_eq!(tail, w["tail"].as_str().expect("tail"), "{n} step {i}");
            pending = tail;
        }
    }
}

#[test]
fn queue_matches_golden() {
    let cases = common::golden("transfers_queue");
    assert!(cases.len() >= 60);
    for case in &cases {
        let n = common::name(case);
        let parallel = case["parallel"].as_i64().expect("parallel");
        let mut q = TransferQueue::new(usize::try_from(parallel).unwrap_or(0));
        let mut ids: Vec<u64> = Vec::new();
        let num = |ids: &[u64], v: &[u64]| -> Vec<u64> {
            v.iter()
                .map(|id| ids.iter().position(|x| x == id).expect("known id") as u64)
                .collect()
        };
        let list = |v: &J| -> Vec<u64> {
            v.as_array()
                .expect("list")
                .iter()
                .map(|x| x.as_u64().expect("index"))
                .collect()
        };
        for (i, (op, want)) in case["ops"]
            .as_array()
            .expect("ops")
            .iter()
            .zip(case["expect"].as_array().expect("expect"))
            .enumerate()
        {
            let at = format!("{n} op {i}");
            match op[0].as_str().expect("op") {
                "add" => ids.push(q.add(job())),
                "set" => {
                    let id = ids[op[1].as_u64().expect("job") as usize];
                    let st = JobState::parse(op[2].as_str().expect("state")).expect("state");
                    // a cleared (detached) job is ignored
                    if let Some(j) = q.get_mut(id) {
                        j.state = st;
                    }
                }
                "startable" => assert_eq!(num(&ids, &q.startable()), list(want), "{at}"),
                "running" => assert_eq!(num(&ids, &q.running()), list(want), "{at}"),
                "clear" => {
                    let gone: Vec<u64> = q.clear_finished().iter().map(|j| j.id).collect();
                    assert_eq!(num(&ids, &gone), list(want), "{at}");
                }
                "active" => assert_eq!(q.active(), want.as_bool().expect("bool"), "{at}"),
                other => panic!("{at}: unknown op {other}"),
            }
        }
    }
}

#[test]
fn real_openssh_meter_lines() {
    let p = parse_progress(
        "big.bin                                         0%    0     0.0KB/s   --:-- ETA",
    )
    .expect("meter");
    assert_eq!(
        (p.percent, p.size.as_str(), p.rate.as_str(), p.eta.as_str()),
        (0, "0", "0.0KB/s", "--:--")
    );
    let p = parse_progress(
        "big.bin                                       100%  381MB -2097151.-9KB/s   00:00",
    )
    .expect("meter");
    assert_eq!(
        (p.percent, p.size.as_str(), p.rate.as_str()),
        (100, "381MB", "")
    );
    assert_eq!(parse_progress("scp: /x: No such file or directory"), None);
}

#[test]
fn queue_runs_at_most_parallel_in_fifo_order() {
    let mut q = TransferQueue::new(2);
    let ids: Vec<u64> = (0..4).map(|_| q.add(job())).collect();
    assert_eq!(q.startable(), ids[..2]);
    q.get_mut(ids[0]).expect("job").state = JobState::Running;
    assert_eq!(q.startable(), ids[1..2]);
    q.get_mut(ids[1]).expect("job").state = JobState::Running;
    assert!(q.startable().is_empty());
    q.get_mut(ids[0]).expect("job").state = JobState::Done;
    assert_eq!(q.startable(), ids[2..3]);
}

#[test]
fn clear_finished_keeps_active() {
    let mut q = TransferQueue::default();
    let (a, b, c) = (q.add(job()), q.add(job()), q.add(job()));
    q.get_mut(a).expect("job").state = JobState::Done;
    q.get_mut(b).expect("job").state = JobState::Cancelled;
    let gone: Vec<u64> = q.clear_finished().iter().map(|j| j.id).collect();
    assert_eq!(gone, [a, b]);
    assert_eq!(q.jobs().len(), 1);
    assert_eq!(q.jobs()[0].id, c);
    assert!(q.active());
}

#[test]
fn debug_hides_the_password_env() {
    let text = format!("{:?}", job());
    assert!(
        !text.contains("hunter2") && !text.contains("SSHPASS"),
        "{text}"
    );
}
