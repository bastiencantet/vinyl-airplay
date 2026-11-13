use crate::{
    crypto::{HapSrp, derive},
    rtsp::Client,
    tlv,
};
use anyhow::{Context, Result, ensure};
use rand::RngCore;

pub async fn transient(client: &mut Client) -> Result<Vec<u8>> {
    let headers = [("X-Apple-HKP", "4".to_string())];
    let m1 = tlv::encode(&[(0, &[0]), (6, &[1]), (0x13, &[0x10])]);
    let response = client
        .request(
            "POST",
            Some("/pair-setup"),
            Some("application/octet-stream"),
            &m1,
            &headers,
        )
        .await?
        .ok()?;
    let m2 = tlv::decode(&response.body)?;
    ensure!(
        m2.get(&6).map(Vec::as_slice) == Some(&[2]),
        "expected pairing state M2"
    );
    let mut private = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut private);
    let srp = HapSrp::new(
        &private,
        m2.get(&2).context("missing SRP salt")?,
        m2.get(&3).context("missing SRP public key")?,
        b"3939",
    )?;
    let m3 = tlv::encode(&[(6, &[3]), (3, &srp.public), (4, &srp.proof)]);
    let response = client
        .request(
            "POST",
            Some("/pair-setup"),
            Some("application/octet-stream"),
            &m3,
            &headers,
        )
        .await?
        .ok()?;
    let m4 = tlv::decode(&response.body)?;
    ensure!(
        m4.get(&6).map(Vec::as_slice) == Some(&[4]),
        "expected pairing state M4"
    );
    let secret = srp.finish(m4.get(&4).context("missing SRP server proof")?)?;
    client.channel.enable(
        &derive(&secret, "Control-Salt", "Control-Write-Encryption-Key"),
        &derive(&secret, "Control-Salt", "Control-Read-Encryption-Key"),
    );
    Ok(secret)
}
