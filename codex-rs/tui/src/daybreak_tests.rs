use super::*;
use codex_protocol::openai_models::ModelAccessPrograms;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn verified_access_eligibility_is_account_scoped_and_fail_closed() {
    for (programs, notice, enrolled) in [
        (json!([]), Notice::Apply, false),
        (
            json!([{"program":"cyber","state":"inactive","grants":[]}]),
            Notice::Apply,
            false,
        ),
        (
            json!([{"program":"cyber","state":"unavailable","grants":[]}]),
            Notice::Limited,
            false,
        ),
        (
            json!([{"program":"cyber","state":"active","grants":[]}]),
            Notice::Limited,
            false,
        ),
        (
            json!([{"program":"cyber","state":"active","grants":[{"level":"tac2"}]}]),
            Notice::Limited,
            true,
        ),
        (
            json!([{"program":"cyber","state":"inactive","grants":[{"level":"tac2"}]}]),
            Notice::Limited,
            false,
        ),
    ] {
        let access: VerifiedAccess = serde_json::from_value(json!({"programs": programs})).unwrap();
        assert_eq!(access.notice(), notice);
        assert_eq!(access.confirmed_enrollment(), enrolled);
    }
}

#[test]
fn catalog_refusal_retry_requires_verified_account_enrollment() {
    let cache = NoticeCache::default();
    assert_eq!(
        notice_for_account(Notice::Disabled, "gpt-5.6-sol", true, &cache),
        Notice::Limited
    );
    cache.set(Notice::Limited).unwrap();
    assert_eq!(
        notice_for_account(Notice::Disabled, "gpt-5.6-sol", true, &cache),
        Notice::Limited
    );
    cache.verified_enrolled.store(true, Ordering::Release);
    assert_eq!(
        notice_for_account(Notice::Disabled, "gpt-5.6-sol", true, &cache),
        Notice::Disabled
    );
    assert_eq!(
        notice_for_account(Notice::Astra, "gpt-6-astra", true, &cache),
        Notice::Astra
    );
    assert_eq!(
        notice_for_account(
            Notice::Disabled,
            "gpt-5.6-sol",
            false,
            &NoticeCache::default()
        ),
        Notice::Disabled
    );
}

#[test]
fn turn_program_uses_catalog_and_preserves_legacy_models() {
    let mut model = crate::test_support::TEST_MODEL_PRESETS[0].clone();
    model.available_access_programs = Some(ModelAccessPrograms {
        cyber: vec![
            CyberAccessProgram::Standard,
            CyberAccessProgram::DaybreakBlue,
        ],
    });
    let models = vec![model.clone()];
    for (name, enabled, expected) in [
        (
            model.model.as_str(),
            true,
            Some(CyberAccessProgram::DaybreakBlue),
        ),
        (
            model.model.as_str(),
            false,
            Some(CyberAccessProgram::Standard),
        ),
        ("unlisted-model", false, None),
        ("unlisted-model", true, None),
    ] {
        assert_eq!(
            program_for_turn(&models, name, /*eligible_account*/ true, enabled),
            Ok(expected)
        );
    }
    assert!(
        program_for_turn(
            &models,
            &model.model,
            /*eligible_account*/ false,
            /*enabled*/ true
        )
        .is_err()
    );
    for (programs, rejects) in [
        (None, false),
        (Some(vec![]), false),
        (Some(vec![CyberAccessProgram::Standard]), true),
    ] {
        model.available_access_programs = programs.map(|cyber| ModelAccessPrograms { cyber });
        let result = program_for_turn(
            std::slice::from_ref(&model),
            &model.model,
            /*eligible_account*/ true,
            /*enabled*/ true,
        );
        if rejects {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(None));
        }
    }
}

#[test]
fn refusal_guidance_uses_the_model_catalog() {
    let mut model = crate::test_support::TEST_MODEL_PRESETS[0].clone();
    model.available_access_programs = Some(ModelAccessPrograms {
        cyber: vec![CyberAccessProgram::Standard],
    });
    for (name, expected) in [
        ("gpt-5.6-sol", Notice::Apply),
        ("gpt-6-astra", Notice::Astra),
        ("gpt-6-astra-wm", Notice::Astra),
    ] {
        model.model = name.into();
        assert_eq!(
            notice_for_setting(
                std::slice::from_ref(&model),
                name,
                /*enabled*/ false,
                /*can_enable_daybreak*/ true,
            ),
            expected
        );
    }
    model
        .available_access_programs
        .as_mut()
        .unwrap()
        .cyber
        .push(CyberAccessProgram::DaybreakBlue);
    assert_eq!(
        notice_for_setting(
            std::slice::from_ref(&model),
            &model.model,
            /*enabled*/ false,
            /*can_enable_daybreak*/ true,
        ),
        Notice::Disabled
    );
    assert_eq!(
        notice_for_setting(
            std::slice::from_ref(&model),
            &model.model,
            /*enabled*/ true,
            /*can_enable_daybreak*/ true,
        ),
        Notice::Enabled
    );
    model.available_access_programs.as_mut().unwrap().cyber =
        vec![CyberAccessProgram::DaybreakBlue];
    assert_eq!(
        notice_for_setting(
            std::slice::from_ref(&model),
            &model.model,
            /*enabled*/ false,
            /*can_enable_daybreak*/ true
        ),
        Notice::Disabled
    );
    let other = model.clone();
    model.model = "gpt-5.6-sol".into();
    model.available_access_programs.as_mut().unwrap().cyber = vec![CyberAccessProgram::Standard];
    assert_eq!(
        notice_for_setting(
            &[model.clone(), other],
            &model.model,
            /*enabled*/ false,
            /*can_enable_daybreak*/ true
        ),
        Notice::Apply
    );
    assert_eq!(
        notice_for_setting(
            &[],
            "gpt-6-astra",
            /*enabled*/ false,
            /*can_enable_daybreak*/ true
        ),
        Notice::Apply
    );
}
