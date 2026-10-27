//! Control (okay::control): ended early, run again, a time given, a scope of tasks and resources
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use okay::control::*;
use okay::services::*;
use okay::typed::*;

#[test]
fn optional_a_program_ended_early() {
    let after = Cell::new(false);
    let got = run(optional(&End, async |cx| {
        let _: () = exit(cx).await;
        after.set(true);
        1
    }));
    assert_eq!(got, Ok(None));
    assert!(!after.get());
    assert_eq!(run(optional(&End, async |_| 2)), Ok(Some(2)));
}

#[test]
fn retry_while_it_fails_the_last_error_kept() {
    let tries = Cell::new(0);
    let got = run(retry(&End, 3, |_: &String| true, async |cx| {
        tries.set(tries.get() + 1);
        raise::<String, i64, _, _>(cx, format!("try {}", tries.get())).await
    }));
    assert_eq!(got, Ok(Err("try 3".to_string())));
    // an error retry_on does not take: no second run
    tries.set(0);
    let got = run(retry(&End, 3, |e: &String| e != "fatal", async |cx| {
        tries.set(tries.get() + 1);
        raise::<String, i64, _, _>(cx, "fatal".to_string()).await
    }));
    assert_eq!((got, tries.get()), (Ok(Err("fatal".to_string())), 1));
}

#[test]
fn a_timeout_on_virtual_time_and_on_the_systems() {
    let cx = Cx(VirtualClock::at(0), End);
    assert_eq!(run(timeout(&cx, 100, async { sleep_ms(&cx, 50).await; 1 })), Ok(Some(1)));
    assert_eq!(run(timeout(&cx, 100, async { sleep_ms(&cx, 500).await; 2 })), Ok(None));

    let cleaned = Rc::new(Cell::new(false));
    struct Cleanup(Rc<Cell<bool>>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let cx = Cx(SystemClock, End);
    let cx = &cx;
    let c = cleaned.clone();
    let got = block_on(timeout(cx, 50, async move {
        let _guard = Cleanup(c);
        sleep_ms(cx, 2_000).await;
        3
    }));
    assert_eq!(got, None);
    assert!(cleaned.get(), "the body dropped where it was: its destructors ran");
    assert_eq!(block_on(timeout(cx, 500, async { sleep_ms(cx, 10).await; 4 })), Some(4));
}

#[test]
fn a_scope_its_tasks_beside_its_body_its_resources_released_the_last_first() {
    let log = RefCell::new(vec![]);
    let cx = Cx(SystemClock, End);
    let got = block_on(scope(async |s| {
        let conn = s.acquire("conn", |c| log.borrow_mut().push(format!("close {c}")));
        s.defer(|| log.borrow_mut().push("deferred".to_string()));
        let a = s.spawn(async { sleep_ms(&cx, 30).await; 1 });
        let b = s.spawn(async { sleep_ms(&cx, 10).await; 2 });
        let forever = s.spawn(async { sleep_ms(&cx, 60_000).await; 3 });
        let _ = &forever;
        let n = a.join().await + b.join().await;
        format!("{conn} {n}")
    }));
    assert_eq!(got, "conn 3");
    // the task left running cancelled at the end; resources released, the last first
    assert_eq!(*log.borrow(), vec!["deferred", "close conn"]);
}

#[test]
fn a_task_cancelled_and_a_scope_left_by_an_error() {
    let dropped = Rc::new(Cell::new(0));
    struct Count(Rc<Cell<i32>>);
    impl Drop for Count {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let released = Cell::new(false);
    let cx = Cx(VirtualClock::at(0), End);
    let d = dropped.clone();
    let got = run(either(&cx, async |cx| {
        scope(async |s| {
            s.defer(|| released.set(true));
            let owned = Count(d);
            let t = s.spawn(async move {
                let _c = owned;
                std::future::pending::<()>().await
            });
            s.cancel(&t);
            raise::<String, (), _, _>(cx, "stop".into()).await
        })
        .await
    }));
    assert_eq!(got, Ok(Err("stop".to_string())));
    assert!(released.get(), "released when the scope was left by the error");
    assert_eq!(dropped.get(), 1, "the cancelled task dropped once");
}

#[test]
fn streams_merged_round_robin() {
    let a = stream(&End, async |cx| emit_all(&cx, [1, 3, 5, 7]).await);
    let b = stream(&End, async |cx| emit_all(&cx, [2, 4]).await);
    assert_eq!(merge(a, b).collect::<Vec<i32>>(), vec![1, 2, 3, 4, 5, 7]);
}
