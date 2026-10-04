use super::*;

fn target_proof(source: &PairingSession, target: &PairingSession) -> Event {
    let hash = derive_transcript_hash(
        &target.session_id,
        &source.pubkey().to_bytes(),
        &target.pubkey().to_bytes(),
        &target.sas_input.expect("SAS input"),
        &target.session_secret,
    );
    target
        .build_event(&PairingMessage::SasConfirm {
            transcript_hash: hex::encode(hash),
        })
        .expect("proof")
}

#[test]
fn code_entry_releases_payload_only_after_peer_proof() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (mut target, offer) = PairingSession::new_target(&qr).expect("target");
    source.handle_offer(&offer).expect("offer");
    assert!(source
        .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
        .is_err());
    let proof = target_proof(&source, &target);
    let response = source
        .handle_target_sas_confirm(&proof)
        .expect("peer proof");
    assert!(source.handle_target_sas_confirm(&proof).is_err(), "replay");
    target.handle_sas_confirm(&response).expect("source proof");
    target.confirm_target_sas().expect("user confirmed");
    let payload = source
        .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
        .expect("payload");
    assert_eq!(
        &*target.handle_payload(&payload).expect("import").1,
        "secret"
    );
    source
        .handle_complete(&target.send_complete().expect("complete"))
        .expect("completion");
    assert_eq!(source.state(), SessionState::Completed);
}

#[test]
fn wrong_peer_cannot_confirm_code_entry() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, offer) = PairingSession::new_target(&qr).expect("target");
    let (other, _) = PairingSession::new_target(&qr).expect("other target");
    source.handle_offer(&offer).expect("offer");
    assert!(source
        .handle_target_sas_confirm(&target_proof(&source, &other))
        .is_err());
    assert_eq!(source.state(), SessionState::Confirming);
    source
        .handle_target_sas_confirm(&target_proof(&source, &target))
        .expect("real peer");
}

#[test]
fn invalid_transcript_aborts_without_releasing_payload() {
    for hash in ["00".repeat(32), "bad".into()] {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (target, offer) = PairingSession::new_target(&qr).expect("target");
        source.handle_offer(&offer).expect("offer");
        let bad = target
            .build_event(&PairingMessage::SasConfirm {
                transcript_hash: hash,
            })
            .expect("proof");
        assert!(matches!(
            source.handle_target_sas_confirm(&bad),
            Err(PairingError::TranscriptMismatch)
        ));
        assert_eq!(source.state(), SessionState::Aborted);
        assert!(source
            .send_payload(PayloadType::Custom, Zeroizing::new("secret".into()))
            .is_err());
    }
}

#[test]
fn unrelated_messages_do_not_consume_confirmation() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, offer) = PairingSession::new_target(&qr).expect("target");
    source.handle_offer(&offer).expect("offer");
    let unexpected = target
        .build_event(&PairingMessage::Complete { success: true })
        .expect("message");
    assert!(source.handle_target_sas_confirm(&unexpected).is_err());
    assert_eq!(source.state(), SessionState::Confirming);
    source
        .handle_target_sas_confirm(&target_proof(&source, &target))
        .expect("proof");
}

#[test]
fn expired_session_rejects_code_entry() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, offer) = PairingSession::new_target(&qr).expect("target");
    source.handle_offer(&offer).expect("offer");
    let proof = target_proof(&source, &target);
    source.created_at = Instant::now() - source.timeout - std::time::Duration::from_secs(1);
    assert!(source.handle_target_sas_confirm(&proof).is_err());
}

#[test]
fn tampered_confirmation_does_not_advance_session() {
    let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
    let (target, offer) = PairingSession::new_target(&qr).expect("target");
    source.handle_offer(&offer).expect("offer");
    let valid = target_proof(&source, &target);
    let mut tampered = valid.clone();
    tampered.content.push('x');
    assert!(source.handle_target_sas_confirm(&tampered).is_err());
    assert_eq!(source.state(), SessionState::Confirming);
    source
        .handle_target_sas_confirm(&valid)
        .expect("valid original proof");
}

#[test]
fn code_entry_capability_is_encrypted_and_uses_one_recipient_tag() {
    for confirmation in [
        None,
        Some("code-entry".to_string()),
        Some("unknown".to_string()),
    ] {
        let (mut source, qr) = PairingSession::new_source("wss://relay.test".into());
        let (target, _) = PairingSession::new_target(&qr).expect("target");
        let offer = target
            .build_event(&PairingMessage::Offer {
                session_id: hex::encode(target.session_id),
                version: 1,
                confirmation: confirmation.clone(),
            })
            .expect("offer");
        assert_eq!(
            offer.tags.len(),
            1,
            "strict pairing relays allow only the recipient tag"
        );
        let (code, entry) = source
            .handle_offer_with_confirmation(&offer)
            .expect("valid offer");
        assert_eq!(entry, confirmation.as_deref() == Some("code-entry"));
        assert_eq!(code, target.sas_code().expect("matching code"));
    }
}
