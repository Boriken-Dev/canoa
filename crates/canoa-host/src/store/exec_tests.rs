use super::*;

fn refused() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, "Failed to assign job")
}

#[test]
fn a_read_the_spawner_refused_is_run_again_and_the_answer_of_the_run_that_worked_is_kept() {
    let mut runs = 0;
    let answer = repeated(3, || {
        runs += 1;
        if runs < 3 {
            Err(refused())
        } else {
            Ok("1.2.3")
        }
    });
    assert_eq!(answer.expect("the third run answered"), "1.2.3");
    assert_eq!(runs, 3);
}

#[test]
fn a_read_that_is_refused_every_time_gives_the_last_refusal_after_its_attempts() {
    let mut runs = 0;
    let answer: Result<(), ProviderError> = repeated(3, || {
        runs += 1;
        Err(refused())
    });
    assert_eq!(runs, 3, "no more than the attempts it was given");
    assert_eq!(
        answer.expect_err("refused").status,
        Status::GUATIAO_ERR_INTERNAL
    );
}

#[test]
fn a_cancel_or_a_deadline_is_never_run_again() {
    let mut runs = 0;
    let answer: Result<(), ProviderError> = repeated(3, || {
        runs += 1;
        Err(ProviderError::new(
            Status::GUATIAO_ERR_GONE,
            "install cancelled",
        ))
    });
    assert_eq!(runs, 1);
    assert_eq!(answer.expect_err("ended").status, Status::GUATIAO_ERR_GONE);
}
