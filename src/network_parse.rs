use std::str::FromStr;

use bitcoin::Network;

use crate::Error;

#[derive(Clone)]
pub struct NetworkParse(Network);
impl FromStr for NetworkParse {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use Network::*;

        let network = match s {
            "bitcoin" | "mainnet" | "main" => NetworkParse(Bitcoin),
            "testnet" | "test" => NetworkParse(Testnet),
            "signet" => NetworkParse(Signet),
            "regtest" => NetworkParse(Regtest),
            _ => return Err(Error::NetworkParseError(s.to_string())),
        };
        Ok(network)
    }
}

impl From<NetworkParse> for Network {
    fn from(value: NetworkParse) -> Self {
        value.0
    }
}
impl From<Network> for NetworkParse {
    fn from(value: Network) -> Self {
        NetworkParse(value)
    }
}
impl AsRef<Network> for NetworkParse {
    fn as_ref(&self) -> &Network {
        &self.0
    }
}
