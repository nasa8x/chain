# Crypto.com Chain Client Integration Tests Suite

## Prerequisites

- Docker: https://www.docker.com/get-started
- Node.js: https://nodejs.org/en/download/
- Yarn: https://yarnpkg.com/en/docs/install
- jq: https://stedolan.github.io/jq/

## Prepare Integration Test

Integration test environment comes with pre-initialized genesis and wallets. Since chain project is under active development, those pre-initialized configurations may have to be prepared again.

### Initialize wallets
This step is needed when Client RPC server and/or the wallet implementation has changed.

1\. To initialize wallet, build the [Crypto.com chain](https://www.github.com/crypto-com/chain) project and run a local node.

2\. Spin up Client RPC server by running
```
$ client-rpc --host 0.0.0.0 --chain-id AB --storage-dir .client-rpc-storage
```
This will generated the necessarily wallets configuration in `.client-rpc-storage`, wallet address list and genesis distribution mapping.

3\. Go to the `integration-tests` folder, run
```
$ ./prepare-wallets.sh
```
4\. Shutdown Client RPC server

5\. Copy `.client-rpc-storage` generated by Client RPC server to folder `integration-tests/docker/chain-preinit/`

6\. Proceed to prepare Docker images

### Prepare Docker images
This step is needed when the genesis app hash generation algorithm has changed. 

To prepare the pre-initialized configuration of the docker images. Go to the `integration-tests` folder, run
```
$ ./prepare-docker.sh
```
This script will build the `dev-utils` and generate the latest App Hash and apply to the Tendermint configuration. It will also update `docker-compose.yml` with the latest commands.

## List of Integration Tests

| Integration Test Suite | Description |
| --- | --- |
| client-rpc | Test related to client RPC server operations |

## How to spin up and teardown Docker Compose Services

Go to the `integration-tests` directory of the chain project, run

### Spin up
```
$ docker-compose  up
```

### Teardown
```
$ docker-compose  down
```

## How to run `client-rpc` Integration Tests Suite

1. Build and spin up Docker containers
2. Go to `client-rpc` directory
```
yarn test
```

Remarks: After each test running, you should always teardown the Docker Compose services to reset the state or otherwise the test result may base on inaccurate state.

## What is included in the Docker containers

### 1. tendermint-preinit
Build the Tendermint image with pre-initialized configuration. `entrypoint.sh` controls which configuration to use on spin up. After spin up there is a initialized distribution of genesis funds and incentive configurations:

#### Build and Runtime

Build Arguments
| Argument | Description | Example |
| --- | --- | --- |
| VERSION | Tendermint version | 0.31.5 |

Runtime Environments
| Environment | Description | Example |
| --- | --- | --- |
| FEE_SCHEMA | `WITH_FEE`: With base fee 1.55, coefficient 0.66; `ZERO_FEE`: Zero fee | ZERO_FEE |

#### Initialized Genesis

Distribution of Genesis Funds

| Address | Amount (in Basic Unit) |
| --- | --- |
| 0xfb135596b941711a1611e59284424d412ee8fd9d | 2500000000000000000 |
| 0x4234ddd8fca1f213180526413042fe2ee6bceac8 | 3000000000000000000 |
| 0x9c58f8fca74d7a555c2c52f3aa49f898dd1fc37b | 3000000000000000000 |
| 0x35f517cab9a37bc31091c2f155d965af84e0bc85 | 500000000000000000 |
| 0x20a0bee429d6907e556205ef9d48ab6fe6a55531 | 500000000000000000 |
| 0x71507ee19cbc0c87ff2b5e05d161efe2aac4ee07 | 500000000000000000 |

Incentive

| Incentive | Address |
| --- | --- |
| Launch Incentive From | 0x35f517cab9a37bc31091c2f155d965af84e0bc85 |
| Launch Incentive To | 0x20a0bee429d6907e556205ef9d48ab6fe6a55531 |
| Long Term Incentive | 0x71507ee19cbc0c87ff2b5e05d161efe2aac4ee07 |


### 2. chain-preinit
Build the `chain-abci` and `client-rpc` in the chain project. A pre-setup wallet will be created:

Pre-setup Wallet

| Wallet Name | First Address |
| --- | --- |
| Default | 0xfb135596b941711a1611e59284424d412ee8fd9d |
| Spend | 0x4234ddd8fca1f213180526413042fe2ee6bceac8 |
| View | 0x9c58f8fca74d7a555c2c52f3aa49f898dd1fc37b |
| Receive | 0x9af90833742a9c5552a0c3336540c8d083c9a79a |

#### Commands

The `chain-abci` and `client-rpc` is available under `/usr/bin/` folder. An example of running these two programs is as follow:
```
/usr/bin/chain-abci --host 0.0.0.0 --port 26658 --chain_id test-chain-y3m1e6-AB --genesis_app_hash BC80954766B2F82A79DFFB776DB6235DD1CB41B4A945056D8DF8B84B2EAE3F3A
```

```
/usr/bin/wait-for-it.sh tendermint:26657 --strict -- /usr/bin/client-rpc --host 0.0.0.0 --port 26659 --chain-id AB --storage-dir .client-rpc-storage --tendermint-url http://tendermint:26657
```

Remarks: Since `client-rpc` requires Tendermint to be running, `wait-for-it.sh` will wait for Tendermint to be available before spinning up `client-rpc`
