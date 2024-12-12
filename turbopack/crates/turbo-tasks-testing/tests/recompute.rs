#![feature(arbitrary_self_types)]
#![feature(arbitrary_self_types_pointers)]
#![allow(clippy::needless_return)] // tokio macro-generated code doesn't respect this

use anyhow::Result;
use turbo_tasks::{State, Vc};
use turbo_tasks_testing::{register, run, Registration};

static REGISTRATION: Registration = register!();

#[tokio::test]
async fn recompute() {
    run(&REGISTRATION, || async {
        let input = ChangingInput {
            state: State::new(1),
        }
        .cell();
        let input2 = ChangingInput {
            state: State::new(10),
        }
        .cell();
        let output = compute(input, input2);
        let read = output.await?;
        assert_eq!(read.state_value, 1);
        assert_eq!(read.state_value2, 10);
        let random_value = read.random_value;

        println!("changing input");
        input.set_state(2).await?;
        let read = output.strongly_consistent().await?;
        assert_eq!(read.state_value, 2);
        assert_ne!(read.random_value, random_value);
        let random_value = read.random_value;

        println!("changing input2");
        input2.set_state(20).await?;
        let read = output.strongly_consistent().await?;
        assert_eq!(read.state_value2, 20);
        assert_ne!(read.random_value, random_value);
        let random_value = read.random_value;

        println!("changing input");
        input.set_state(5).await?;
        let read = output.strongly_consistent().await?;
        assert_eq!(read.state_value, 5);
        assert_eq!(read.state_value2, 42);
        assert_ne!(read.random_value, random_value);
        let random_value = read.random_value;

        println!("changing input2");
        input2.set_state(30).await?;
        let read = output.strongly_consistent().await?;
        assert_eq!(read.random_value, random_value);

        anyhow::Ok(())
    })
    .await
    .unwrap()
}

#[turbo_tasks::value]
struct ChangingInput {
    state: State<u32>,
}

#[turbo_tasks::value_impl]
impl ChangingInput {
    // HACK: This write has to be a separate task to avoid creating a cycle in the parent/child call
    // tree, since `State::set`/`State::get` marks the writer task as a child of the reader task.
    #[turbo_tasks::function]
    fn set_state(&self, value: u32) {
        self.state.set(value);
    }
}

#[turbo_tasks::value]
struct Output {
    state_value: u32,
    state_value2: u32,
    random_value: u32,
}

#[turbo_tasks::function]
async fn compute(input: Vc<ChangingInput>, input2: Vc<ChangingInput>) -> Result<Vc<Output>> {
    let state_value = *input.await?.state.get();
    let state_value2 = if state_value < 5 {
        *compute2(input2).await?
    } else {
        42
    };
    let random_value = rand::random();

    Ok(Output {
        state_value,
        state_value2,
        random_value,
    }
    .cell())
}

#[turbo_tasks::function]
async fn compute2(input: Vc<ChangingInput>) -> Result<Vc<u32>> {
    let state_value = *input.await?.state.get();
    Ok(Vc::cell(state_value))
}
