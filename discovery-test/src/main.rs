use std::env::temp_dir;

use distrans_peer::{new_routing_context, Error, Observable, Peer, Veilid};
use tokio::{select, spawn};
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FourCC, KeyPair, PublicKey, RoutingContext,
    SecretKey, TypedKey, TypedKeyPair, VeilidAPIError, CRYPTO_KIND_VLD0, PUBLIC_KEY_LENGTH,
};

static RENDEZVOUS_KIND: FourCC = CRYPTO_KIND_VLD0;

pub struct Rendezvous {
    routing_context: RoutingContext,
}

impl Rendezvous {
    pub fn new(routing_context: RoutingContext) -> Rendezvous {
        Rendezvous { routing_context }
    }

    pub async fn open_or_create(
        &self,
        schema: DHTSchema,
        digest: &[u8; PUBLIC_KEY_LENGTH],
    ) -> Result<(DHTRecordDescriptor, TypedKeyPair), VeilidAPIError> {
        let rendezvous_keypair = Self::derive_rendezvous_key(digest);
        match self
            .routing_context
            .create_dht_record(
                schema.clone(),
                Some(rendezvous_keypair.value),
                Some(rendezvous_keypair.kind),
            )
            .await
        {
            Ok(rec) => return Ok((rec, rendezvous_keypair)),
            Err(VeilidAPIError::Internal { message }) => {
                if message != "record already exists" {
                    return Err(VeilidAPIError::Internal { message });
                }
            }
            Err(e) => return Err(e),
        };

        // Key already exists in DHT, open it.
        let dht_key = self.derive_dht_key(&rendezvous_keypair.value.key, &schema)?;
        Ok((
            self.routing_context
                .open_dht_record(dht_key, Some(rendezvous_keypair.value))
                .await?,
            rendezvous_keypair,
        ))
    }

    fn derive_rendezvous_key(digest: &[u8; 32]) -> TypedKeyPair {
        let digest_key = ed25519_dalek::SigningKey::from_bytes(&digest);
        let keypair = KeyPair::new(
            PublicKey::new(digest_key.verifying_key().as_bytes().to_owned()),
            SecretKey::new(digest_key.as_bytes().to_owned()),
        );
        TypedKeyPair::new(RENDEZVOUS_KIND, keypair)
    }

    fn derive_dht_key(
        &self,
        owner_key: &PublicKey,
        schema: &DHTSchema,
    ) -> Result<TypedKey, VeilidAPIError> {
        let schema_data = schema.compile();
        let mut hash_data = Vec::<u8>::with_capacity(PUBLIC_KEY_LENGTH + 4 + schema_data.len());
        hash_data.extend_from_slice(&RENDEZVOUS_KIND.0);
        hash_data.extend_from_slice(&owner_key.bytes);
        hash_data.extend_from_slice(&schema_data);
        let hash = self
            .routing_context
            .api()
            .crypto()?
            .get(RENDEZVOUS_KIND)
            .ok_or(VeilidAPIError::unimplemented(
                "failed to resolve cryptosystem",
            ))?
            .generate_hash(&hash_data);
        Ok(TypedKey::new(RENDEZVOUS_KIND, hash))
    }
}

#[tokio::main]
async fn main() {
    run().await.expect("ok")
}

async fn run() -> Result<(), Error> {
    let state_dir = temp_dir();
    let (routing_context, update_tx, _) =
        new_routing_context(state_dir.as_os_str().to_str().unwrap()).await?;
    let mut peer = Observable::new(Veilid::new(routing_context.clone(), update_tx).await?);
    let mut progress = peer.subscribe_peer_progress();
    spawn(async move {
        loop {
            select! {
                res = progress.changed() => {
                    if let Ok(()) = res {
                        let p = progress.borrow_and_update();
                        println!("{:?}", p);
                    }
                }
            }
        }
    });
    peer.reset().await?;

    let mut digest = [0u8; 32];
    hex::decode_to_slice(
        "c60fcdc8671c0724ff2f775f3dfe363bddce586505118163120c1b3def34d53a",
        &mut digest[0..32],
    )
    .unwrap();

    let rdv = Rendezvous::new(routing_context.clone());
    let (dht_rec, owner) = rdv
        .open_or_create(DHTSchema::DFLT(DHTSchemaDFLT::new(16)?), &digest)
        .await?;
    routing_context
        .set_dht_value(
            dht_rec.key().to_owned(),
            0,
            "hello world".to_owned().into(),
            Some(owner.value),
        )
        .await
        .unwrap();
    println!("Hello, world!");
    Ok(())
}
