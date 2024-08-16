use fuel_crypto::{
    fuel_types::ChainId,
    SecretKey,
};
use fuel_tx::{
    policies::Policies,
    Input,
    Signable,
    Transaction,
    Upgrade,
    UpgradePurpose,
    Upload,
    UploadSubsection,
    Witness,
};
use genesis_fuel_core_bin::FuelService as GenesisFuelService;
use genesis_fuel_core_client::client::FuelClient as GenesisClient;
use genesis_fuel_core_services::Service as _;
use latest_fuel_core_bin::FuelService as LatestFuelService;
use latest_fuel_core_client::client::FuelClient as LatestClient;
use libp2p::PeerId;
use rand::{
    prelude::StdRng,
    Rng,
};
use std::str::FromStr;

macro_rules! define_core_driver {
    ($bin_crate:ident, $service:ident, $client:ident, $name:ident) => {
        pub struct $name {
            /// This must be before the db_dir as the drop order matters here.
            pub node: $service,
            pub db_dir: tempfile::TempDir,
            pub client: $client,
        }

        impl $name {
            pub async fn spawn(extra_args: &[&str]) -> anyhow::Result<Self> {
                use clap::Parser;
                use tempfile::tempdir;

                // Generate temp params
                let db_dir = tempdir()?;

                let mut args = vec![
                    "_IGNORED_",
                    "--db-path",
                    db_dir.path().to_str().unwrap(),
                    "--port",
                    "0",
                ];
                args.extend(extra_args);

                let node = $bin_crate::cli::run::get_service(
                    $bin_crate::cli::run::Command::parse_from(args),
                )?;

                node.start_and_await().await?;

                let client = $client::from(node.shared.graph_ql.bound_address);
                Ok(Self {
                    node,
                    db_dir,
                    client,
                })
            }
        }
    };
}

define_core_driver!(
    genesis_fuel_core_bin,
    GenesisFuelService,
    GenesisClient,
    GenesisFuelCoreDriver
);

define_core_driver!(
    latest_fuel_core_bin,
    LatestFuelService,
    LatestClient,
    LatestFuelCoreDriver
);

pub const IGNITION_SNAPSHOT: &str = "./chain-configurations/ignition";
pub const POA_SECRET_KEY: &str =
    "e3d6eb39607650e22f0befa26d52e921d2e7924d0e165f38ffa8d9d0ac73de93";
pub const PRIVILEGED_ADDRESS_KEY: &str =
    "dcbe36d8e890d7489b6e1be442eab98ae2fdbb5c7d77e1f9e1e12a545852304f";
pub const BASE_ASSET_ID: &str =
    "0xf8f8b6283d7fa5b672b530cbb84fcccb4ff8dc40f8176ef4544ddb1f1952ad07";

pub fn default_multiaddr(port: &str, peer_id: PeerId) -> String {
    format!("/ip4/127.0.0.1/tcp/{}/p2p/{}", port, peer_id)
}

pub const SUBSECTION_SIZE: usize = 64 * 1024;

pub fn valid_input(secret_key: &SecretKey, rng: &mut StdRng, amount: u64) -> Input {
    let pk = secret_key.public_key();
    let owner = Input::owner(&pk);
    Input::coin_signed(
        rng.gen(),
        owner,
        amount,
        BASE_ASSET_ID.parse().unwrap(),
        Default::default(),
        Default::default(),
    )
}

pub fn transactions_from_subsections(
    rng: &mut StdRng,
    subsections: Vec<UploadSubsection>,
    amount: u64,
) -> Vec<Upload> {
    subsections
        .into_iter()
        .map(|subsection| {
            let secret_key: SecretKey =
                SecretKey::from_str(PRIVILEGED_ADDRESS_KEY).unwrap();
            let mut tx = Transaction::upload_from_subsection(
                subsection,
                Policies::new().with_max_fee(amount),
                vec![valid_input(&secret_key, rng, amount)],
                vec![],
                vec![Witness::default()],
            );
            tx.sign_inputs(&secret_key, &ChainId::new(0));

            tx
        })
        .collect::<Vec<_>>()
}

pub fn upgrade_transaction(
    purpose: UpgradePurpose,
    rng: &mut StdRng,
    amount: u64,
) -> Upgrade {
    let secret_key: SecretKey = SecretKey::from_str(PRIVILEGED_ADDRESS_KEY).unwrap();
    let mut tx = Transaction::upgrade(
        purpose,
        Policies::new().with_max_fee(100000),
        vec![valid_input(&secret_key, rng, amount)],
        vec![],
        vec![Witness::default()],
    );
    tx.sign_inputs(&secret_key, &ChainId::new(0));
    tx
}
