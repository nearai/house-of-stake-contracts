use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_STAKING_WASM: &str = "res/local/staking_contract.wasm";
const DEFAULT_STAKING_TEST_WASM: &str = "res/local/staking_contract_test.wasm";
const DEFAULT_MOCK_POOL_WASM: &str = "res/local/mock_staking_pool_contract.wasm";

#[derive(Parser)]
#[command(
    name = "staking-cli",
    about = "Bootstrap deployment CLI for staking-contract"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Deploy a fresh staking contract or replace code without running init.
    Deploy(DeployArgs),
    /// Apply validator and catalog bootstrap configuration.
    Configure(ConfigureArgs),
    /// Run read-only post-deploy checks.
    Verify(VerifyArgs),
}

#[derive(Args)]
struct DeployArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Fresh deploy with `new(config)`.
    #[arg(long, conflicts_with = "code_only")]
    fresh: bool,
    /// Code-only deploy with `without-init-call`; never runs `migrate_state()`.
    #[arg(long)]
    code_only: bool,
    /// Contract owner for fresh deploy init. Defaults to `--account`.
    #[arg(long)]
    owner: Option<String>,
    /// WASM path. Defaults to normal or test-feature artifact based on `--test-feature`.
    #[arg(long)]
    wasm: Option<PathBuf>,
    /// Use `res/local/staking_contract_test.wasm` by default and verify test-only methods.
    #[arg(long)]
    test_feature: bool,
}

#[derive(Args)]
struct ConfigureArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Mock pool WASM used by validators with `deploy_mock_pool: true`.
    #[arg(long, default_value = DEFAULT_MOCK_POOL_WASM)]
    mock_pool_wasm: PathBuf,
}

#[derive(Args)]
struct VerifyArgs {
    #[command(flatten)]
    common: ReadOnlyCommonArgs,
    /// Also check the test-feature-only `get_block_timestamp` view.
    #[arg(long)]
    test_feature: bool,
}

#[derive(Args, Clone)]
struct CommonArgs {
    #[command(flatten)]
    readonly: ReadOnlyCommonArgs,
    /// Send transactions. Without this flag, mutating commands only print planned transactions.
    #[arg(long)]
    send: bool,
    /// Allow mutating commands on mainnet.
    #[arg(long)]
    yes_mainnet: bool,
}

#[derive(Args, Clone)]
struct ReadOnlyCommonArgs {
    /// Network passed to `near network-config`.
    #[arg(long, value_enum, default_value = "testnet")]
    network: Network,
    /// Staking contract account id.
    #[arg(long)]
    account: Option<String>,
    /// Optional bootstrap config JSON.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Signer account id. Defaults to the command-specific owner or staking account.
    #[arg(long)]
    signer: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Network {
    Testnet,
    Mainnet,
}

impl Network {
    fn as_near_network(self) -> &'static str {
        match self {
            Self::Testnet => "testnet",
            Self::Mainnet => "mainnet",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct BootstrapConfig {
    #[serde(default)]
    staking: StakingConfig,
    #[serde(default)]
    init: InitConfig,
    #[serde(default)]
    validators: Vec<ValidatorConfig>,
    #[serde(default)]
    products: Vec<ProductConfig>,
    #[serde(default)]
    verify: VerifyConfig,
}

#[derive(Debug, Default, Deserialize)]
struct StakingConfig {
    account_id: Option<String>,
    owner_account_id: Option<String>,
    signer_account_id: Option<String>,
    wasm: Option<PathBuf>,
    test_wasm: Option<PathBuf>,
    test_feature: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct InitConfig {
    #[serde(default)]
    guardians: Vec<String>,
    #[serde(default = "default_min_lock_duration_ns")]
    min_lock_duration_ns: String,
    #[serde(default = "default_max_lock_duration_ns")]
    max_lock_duration_ns: String,
    #[serde(default = "default_epoch_unstake_settle_epochs")]
    epoch_unstake_settle_epochs: u64,
    #[serde(default = "default_min_storage_deposit")]
    min_storage_deposit: String,
    #[serde(default)]
    per_lock_storage_stake: String,
    #[serde(default)]
    per_farm_position_storage_stake: String,
    #[serde(default)]
    per_purchase_storage_stake: String,
    #[serde(default = "default_min_lock_amount")]
    min_lock_amount: String,
}

impl Default for InitConfig {
    fn default() -> Self {
        Self {
            guardians: Vec::new(),
            min_lock_duration_ns: default_min_lock_duration_ns(),
            max_lock_duration_ns: default_max_lock_duration_ns(),
            epoch_unstake_settle_epochs: default_epoch_unstake_settle_epochs(),
            min_storage_deposit: default_min_storage_deposit(),
            per_lock_storage_stake: "0".to_string(),
            per_farm_position_storage_stake: "0".to_string(),
            per_purchase_storage_stake: "0".to_string(),
            min_lock_amount: default_min_lock_amount(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ValidatorConfig {
    validator_id: String,
    #[serde(default)]
    owner_account_id: Option<String>,
    #[serde(default)]
    deploy_mock_pool: bool,
}

#[derive(Debug, Deserialize)]
struct ProductConfig {
    validator_id: String,
    #[serde(default)]
    owner_account_id: Option<String>,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    prices: Vec<PriceConfig>,
}

#[derive(Debug, Deserialize)]
struct PriceConfig {
    name: String,
    #[serde(default)]
    description: String,
    amount: String,
    #[serde(default = "default_price_type")]
    price_type: String,
    #[serde(default)]
    billing_period: Option<String>,
    #[serde(default)]
    lock_factor_near_months: String,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    set_default: bool,
}

#[derive(Debug, Default, Deserialize)]
struct VerifyConfig {
    #[serde(default)]
    test_feature: bool,
    #[serde(default = "default_view_limit")]
    view_limit: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Deploy(args) => deploy(args),
        Commands::Configure(args) => configure(args),
        Commands::Verify(args) => verify(args),
    }
}

fn deploy(args: DeployArgs) -> Result<()> {
    let config = load_config(args.common.readonly.config.as_deref())?;
    let account_id = resolve_account(&args.common.readonly, &config)?;
    let owner = args
        .owner
        .or_else(|| config.staking.owner_account_id.clone())
        .unwrap_or_else(|| account_id.clone());
    let signer = args
        .common
        .readonly
        .signer
        .clone()
        .or_else(|| config.staking.signer_account_id.clone())
        .unwrap_or_else(|| owner.clone());
    let test_feature = args.test_feature || config.staking.test_feature.unwrap_or(false);
    let wasm = args
        .wasm
        .or_else(|| {
            if test_feature {
                config.staking.test_wasm.clone()
            } else {
                config.staking.wasm.clone()
            }
        })
        .unwrap_or_else(|| {
            if test_feature {
                DEFAULT_STAKING_TEST_WASM.into()
            } else {
                DEFAULT_STAKING_WASM.into()
            }
        });
    let mode = deploy_mode(args.fresh, args.code_only)?;
    let ctx = MutContext::new(args.common, account_id.clone(), signer)?;
    guard_mainnet(&ctx)?;
    require_file(&wasm)?;

    println!(
        "network:        {}",
        ctx.common.readonly.network.as_near_network()
    );
    println!("account:        {account_id}");
    println!("signer:         {}", ctx.signer);
    println!("mode:           {mode}");
    println!("wasm:           {}", wasm.display());
    println!("wasm sha256:    {}", wasm_sha256(&wasm)?);
    println!("test feature:   {test_feature}");

    let mut cmd = NearCommand::new(ctx.common.readonly.network);
    cmd.arg("contract")
        .arg("deploy")
        .arg(&account_id)
        .arg("use-file")
        .arg(path_arg(&wasm));

    match mode {
        DeployMode::Fresh => {
            let init_args = json!({ "config": init_json(&owner, &config.init) });
            cmd.arg("with-init-call")
                .arg("new")
                .arg("json-args")
                .arg(init_args.to_string())
                .arg("prepaid-gas")
                .arg("100.0 Tgas")
                .arg("attached-deposit")
                .arg("0 NEAR");
        }
        DeployMode::CodeOnly => {
            cmd.arg("without-init-call");
        }
    }

    cmd.arg("network-config")
        .arg(ctx.common.readonly.network.as_near_network())
        .arg("sign-with-keychain")
        .arg("send");
    run_tx(&ctx, cmd)?;

    if ctx.common.send {
        verify(VerifyArgs {
            common: ctx.common.readonly,
            test_feature,
        })?;
    }
    Ok(())
}

fn configure(args: ConfigureArgs) -> Result<()> {
    let config = load_config(args.common.readonly.config.as_deref())?;
    let account_id = resolve_account(&args.common.readonly, &config)?;
    let signer = args
        .common
        .readonly
        .signer
        .clone()
        .or_else(|| config.staking.signer_account_id.clone())
        .or_else(|| config.staking.owner_account_id.clone())
        .unwrap_or_else(|| account_id.clone());
    let ctx = MutContext::new(args.common, account_id.clone(), signer)?;
    guard_mainnet(&ctx)?;

    println!("network: {}", ctx.common.readonly.network.as_near_network());
    println!("account: {account_id}");
    println!("signer:  {}", ctx.signer);
    println!("send:    {}", ctx.common.send);

    for validator in &config.validators {
        configure_validator(&ctx, &account_id, validator, &args.mock_pool_wasm)?;
    }

    let mut product_cache = HashMap::new();
    for product in &config.products {
        let product_id = configure_product(&ctx, &account_id, product, &mut product_cache)?;
        for price in &product.prices {
            let price_id = configure_price(&ctx, &account_id, &product_id, product, price)?;
            if price.set_default {
                set_default_price(&ctx, &account_id, product, &product_id, &price_id)?;
            }
        }
    }

    if ctx.common.send {
        verify(VerifyArgs {
            test_feature: config.verify.test_feature
                || config.staking.test_feature.unwrap_or(false),
            common: ctx.common.readonly,
        })?;
    }
    Ok(())
}

fn verify(args: VerifyArgs) -> Result<()> {
    let config = load_config(args.common.config.as_deref())?;
    let account_id = resolve_account(&args.common, &config)?;
    let network = args.common.network;
    let limit = config.verify.view_limit;
    let test_feature = args.test_feature
        || config.verify.test_feature
        || config.staking.test_feature.unwrap_or(false);

    println!("network: {}", network.as_near_network());
    println!("account: {account_id}");

    let version = view_json(network, &account_id, "get_version", json!({}))?;
    println!("version: {version}");

    let contract_config = view_json(network, &account_id, "get_config", json!({}))?;
    println!(
        "owner:   {}",
        contract_config
            .get("owner_account_id")
            .and_then(Value::as_str)
            .unwrap_or("<missing>")
    );

    let validators = view_json(
        network,
        &account_id,
        "get_validators",
        json!({ "from_index": 0, "limit": limit }),
    )?;
    println!("validators: {}", validators.as_array().map_or(0, Vec::len));

    let products = view_json(
        network,
        &account_id,
        "get_products",
        json!({ "from_index": 0, "limit": limit }),
    )?;
    println!("products:   {}", products.as_array().map_or(0, Vec::len));

    let bounds = view_json(network, &account_id, "storage_balance_bounds", json!({}))?;
    println!("storage balance bounds: {bounds}");

    if test_feature {
        let ts = view_json(network, &account_id, "get_block_timestamp", json!({}))?;
        println!("test clock: {ts}");
    }

    Ok(())
}

fn configure_validator(
    ctx: &MutContext,
    staking_account: &str,
    validator: &ValidatorConfig,
    mock_pool_wasm: &Path,
) -> Result<()> {
    if validator.deploy_mock_pool {
        require_file(mock_pool_wasm)?;
        let owner = validator
            .owner_account_id
            .as_deref()
            .unwrap_or(ctx.signer.as_str());
        let init_args = json!({ "owner_id": owner });
        let mut cmd = NearCommand::new(ctx.common.readonly.network);
        cmd.arg("contract")
            .arg("deploy")
            .arg(&validator.validator_id)
            .arg("use-file")
            .arg(path_arg(mock_pool_wasm))
            .arg("with-init-call")
            .arg("new")
            .arg("json-args")
            .arg(init_args.to_string())
            .arg("prepaid-gas")
            .arg("50.0 Tgas")
            .arg("attached-deposit")
            .arg("0 NEAR")
            .arg("network-config")
            .arg(ctx.common.readonly.network.as_near_network())
            .arg("sign-with-keychain")
            .arg("send");
        run_tx(ctx, cmd)?;
    }

    let existing = view_json(
        ctx.common.readonly.network,
        staking_account,
        "get_validator",
        json!({ "validator_id": validator.validator_id }),
    )?;
    if !existing.is_null() {
        println!("validator already exists: {}", validator.validator_id);
        return Ok(());
    }

    near_tx(
        ctx,
        staking_account,
        "add_validator",
        json!({ "validator_id": validator.validator_id }),
        "50.0 Tgas",
        "1 yoctoNEAR",
        ctx.signer.as_str(),
    )
}

fn configure_product(
    ctx: &MutContext,
    staking_account: &str,
    product: &ProductConfig,
    cache: &mut HashMap<(String, String), String>,
) -> Result<String> {
    let key = (product.validator_id.clone(), product.name.clone());
    if let Some(product_id) = cache.get(&key) {
        return Ok(product_id.clone());
    }

    if let Some(product_id) = find_product(ctx.common.readonly.network, staking_account, product)? {
        println!("product already exists: {} ({product_id})", product.name);
        cache.insert(key, product_id.clone());
        return Ok(product_id);
    }

    let signer = product
        .owner_account_id
        .as_deref()
        .unwrap_or(ctx.signer.as_str());
    near_tx(
        ctx,
        staking_account,
        "create_product",
        json!({
            "validator_id": product.validator_id,
            "name": product.name,
            "description": product.description,
        }),
        "200.0 Tgas",
        "1 yoctoNEAR",
        signer,
    )?;

    if !ctx.common.send {
        let placeholder = format!("<product id for {}>", product.name);
        cache.insert(key, placeholder.clone());
        return Ok(placeholder);
    }

    let product_id = find_product(ctx.common.readonly.network, staking_account, product)?
        .ok_or_else(|| anyhow!("created product was not found by name: {}", product.name))?;
    cache.insert(key, product_id.clone());
    Ok(product_id)
}

fn configure_price(
    ctx: &MutContext,
    staking_account: &str,
    product_id: &str,
    product: &ProductConfig,
    price: &PriceConfig,
) -> Result<String> {
    if let Some(price_id) = find_price(
        ctx.common.readonly.network,
        staking_account,
        product_id,
        price,
    )? {
        println!("price already exists: {} ({price_id})", price.name);
        return Ok(price_id);
    }

    let signer = product
        .owner_account_id
        .as_deref()
        .unwrap_or(ctx.signer.as_str());
    near_tx(
        ctx,
        staking_account,
        "create_price",
        json!({
            "product_id": product_id,
            "name": price.name,
            "description": price.description,
            "amount": price.amount,
            "price_type": price.price_type,
            "billing_period": price.billing_period,
            "lock_factor_near_months": price.lock_factor_near_months,
            "metadata": price.metadata,
        }),
        "200.0 Tgas",
        "1 yoctoNEAR",
        signer,
    )?;

    if !ctx.common.send {
        return Ok(format!("<price id for {}>", price.name));
    }

    find_price(
        ctx.common.readonly.network,
        staking_account,
        product_id,
        price,
    )?
    .ok_or_else(|| anyhow!("created price was not found by name: {}", price.name))
}

fn set_default_price(
    ctx: &MutContext,
    staking_account: &str,
    product: &ProductConfig,
    product_id: &str,
    price_id: &str,
) -> Result<()> {
    if price_id.starts_with('<') {
        println!("skip default price dry-run placeholder for product {product_id}");
        return Ok(());
    }
    let stored = view_json(
        ctx.common.readonly.network,
        staking_account,
        "get_product",
        json!({ "product_id": product_id }),
    )?;
    if stored.get("default_price_id").and_then(Value::as_str) == Some(price_id) {
        println!("default price already set: {product_id} -> {price_id}");
        return Ok(());
    }
    let signer = product
        .owner_account_id
        .as_deref()
        .unwrap_or(ctx.signer.as_str());
    near_tx(
        ctx,
        staking_account,
        "set_product_default_price",
        json!({ "product_id": product_id, "price_id": price_id }),
        "200.0 Tgas",
        "1 yoctoNEAR",
        signer,
    )
}

fn find_product(
    network: Network,
    staking_account: &str,
    product: &ProductConfig,
) -> Result<Option<String>> {
    let products = view_json(
        network,
        staking_account,
        "get_products",
        json!({ "from_index": 0, "limit": 200 }),
    )?;
    let Some(items) = products.as_array() else {
        bail!("get_products did not return an array");
    };
    Ok(items
        .iter()
        .rev()
        .find(|item| {
            item.get("validator_id").and_then(Value::as_str) == Some(product.validator_id.as_str())
                && item.get("name").and_then(Value::as_str) == Some(product.name.as_str())
                && item.get("status").and_then(Value::as_str) == Some("Active")
        })
        .and_then(|item| item.get("product_id").and_then(Value::as_str))
        .map(ToOwned::to_owned))
}

fn find_price(
    network: Network,
    staking_account: &str,
    product_id: &str,
    price: &PriceConfig,
) -> Result<Option<String>> {
    if product_id.starts_with('<') {
        return Ok(None);
    }
    let product = view_json(
        network,
        staking_account,
        "get_product",
        json!({ "product_id": product_id }),
    )?;
    let Some(price_ids) = product.get("price_ids").and_then(Value::as_array) else {
        return Ok(None);
    };
    for price_id in price_ids.iter().filter_map(Value::as_str) {
        let candidate = view_json(
            network,
            staking_account,
            "get_price",
            json!({ "price_id": price_id }),
        )?;
        if candidate.get("name").and_then(Value::as_str) == Some(price.name.as_str())
            && candidate.get("amount").and_then(Value::as_str) == Some(price.amount.as_str())
            && candidate.get("price_type").and_then(Value::as_str)
                == Some(price.price_type.as_str())
            && candidate
                .get("lock_factor_near_months")
                .and_then(Value::as_str)
                == Some(price.lock_factor_near_months.as_str())
            && candidate.get("status").and_then(Value::as_str) == Some("Active")
        {
            return Ok(Some(price_id.to_string()));
        }
    }
    Ok(None)
}

fn near_tx(
    ctx: &MutContext,
    contract_id: &str,
    method: &str,
    args: Value,
    gas: &str,
    deposit: &str,
    signer: &str,
) -> Result<()> {
    let mut cmd = NearCommand::new(ctx.common.readonly.network);
    cmd.arg("contract")
        .arg("call-function")
        .arg("as-transaction")
        .arg(contract_id)
        .arg(method)
        .arg("json-args")
        .arg(args.to_string())
        .arg("prepaid-gas")
        .arg(gas)
        .arg("attached-deposit")
        .arg(deposit)
        .arg("sign-as")
        .arg(signer)
        .arg("network-config")
        .arg(ctx.common.readonly.network.as_near_network())
        .arg("sign-with-keychain")
        .arg("send");
    run_tx(ctx, cmd)
}

fn view_json(network: Network, contract_id: &str, method: &str, args: Value) -> Result<Value> {
    let output = Command::new("near")
        .arg("--quiet")
        .arg("contract")
        .arg("call-function")
        .arg("as-read-only")
        .arg(contract_id)
        .arg(method)
        .arg("json-args")
        .arg(args.to_string())
        .arg("network-config")
        .arg(network.as_near_network())
        .arg("now")
        .output()
        .with_context(|| "failed to run near CLI")?;
    if !output.status.success() {
        bail!(
            "near view failed for {method}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_near_json(&stdout)
        .with_context(|| format!("failed to parse near output for {method}: {stdout}"))
}

fn parse_near_json(output: &str) -> Result<Value> {
    let trimmed = output.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    for (start, ch) in trimmed.char_indices() {
        if !matches!(ch, '{' | '[' | '"' | 'n' | 't' | 'f' | '-' | '0'..='9') {
            continue;
        }
        for end in trimmed
            .char_indices()
            .map(|(idx, _)| idx)
            .chain(std::iter::once(trimmed.len()))
            .filter(|end| *end > start)
        {
            if let Ok(value) = serde_json::from_str(&trimmed[start..end]) {
                return Ok(value);
            }
        }
    }
    bail!("no JSON value found")
}

fn run_tx(ctx: &MutContext, cmd: NearCommand) -> Result<()> {
    println!("+ {}", cmd.display());
    if !ctx.common.send {
        return Ok(());
    }
    let status = Command::new("near")
        .args(cmd.args)
        .status()
        .with_context(|| "failed to run near CLI")?;
    if !status.success() {
        bail!("near transaction failed with status {status}");
    }
    Ok(())
}

struct NearCommand {
    args: Vec<OsString>,
}

impl NearCommand {
    fn new(_network: Network) -> Self {
        Self {
            args: vec![OsString::from("--quiet")],
        }
    }

    fn arg(&mut self, arg: impl Into<OsString>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    fn display(&self) -> String {
        let mut parts = vec!["near".to_string()];
        parts.extend(
            self.args
                .iter()
                .map(|arg| shell_quote(&arg.to_string_lossy())),
        );
        parts.join(" ")
    }
}

struct MutContext {
    common: CommonArgs,
    signer: String,
}

impl MutContext {
    fn new(common: CommonArgs, _account_id: String, signer: String) -> Result<Self> {
        Ok(Self { common, signer })
    }
}

#[derive(Clone, Copy)]
enum DeployMode {
    Fresh,
    CodeOnly,
}

impl std::fmt::Display for DeployMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fresh => write!(f, "fresh"),
            Self::CodeOnly => write!(f, "code-only"),
        }
    }
}

fn deploy_mode(fresh: bool, code_only: bool) -> Result<DeployMode> {
    match (fresh, code_only) {
        (true, false) => Ok(DeployMode::Fresh),
        (false, true) => Ok(DeployMode::CodeOnly),
        (false, false) => bail!("choose exactly one deploy mode: --fresh or --code-only"),
        (true, true) => bail!("--fresh and --code-only are mutually exclusive"),
    }
}

fn guard_mainnet(ctx: &MutContext) -> Result<()> {
    if ctx.common.send && ctx.common.readonly.network == Network::Mainnet && !ctx.common.yes_mainnet
    {
        bail!("mainnet mutations require --yes-mainnet");
    }
    Ok(())
}

fn load_config(path: Option<&Path>) -> Result<BootstrapConfig> {
    let Some(path) = path else {
        return Ok(BootstrapConfig::default());
    };
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse config {}", path.display()))
}

fn resolve_account(args: &ReadOnlyCommonArgs, config: &BootstrapConfig) -> Result<String> {
    args.account
        .clone()
        .or_else(|| config.staking.account_id.clone())
        .ok_or_else(|| anyhow!("missing staking account; pass --account or set staking.account_id"))
}

fn init_json(owner: &str, init: &InitConfig) -> Value {
    json!({
        "owner_account_id": owner,
        "proposed_new_owner_account_id": null,
        "guardians": init.guardians,
        "min_lock_duration_ns": init.min_lock_duration_ns,
        "max_lock_duration_ns": init.max_lock_duration_ns,
        "epoch_unstake_settle_epochs": init.epoch_unstake_settle_epochs,
        "min_storage_deposit": init.min_storage_deposit,
        "per_lock_storage_stake": init.per_lock_storage_stake,
        "per_farm_position_storage_stake": init.per_farm_position_storage_stake,
        "per_purchase_storage_stake": init.per_purchase_storage_stake,
        "min_lock_amount": init.min_lock_amount,
    })
}

fn wasm_sha256(path: &Path) -> Result<String> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(data)))
}

fn require_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("missing file: {}", path.display());
    }
    Ok(())
}

fn path_arg(path: &Path) -> OsString {
    path.as_os_str().to_os_string()
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=@".contains(ch))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn default_min_lock_duration_ns() -> String {
    "1".to_string()
}

fn default_max_lock_duration_ns() -> String {
    "63072000000000000".to_string()
}

fn default_epoch_unstake_settle_epochs() -> u64 {
    4
}

fn default_min_storage_deposit() -> String {
    "10000000000000000000000".to_string()
}

fn default_min_lock_amount() -> String {
    "1000000000000000000000000".to_string()
}

fn default_price_type() -> String {
    "OneOff".to_string()
}

fn default_view_limit() -> u64 {
    20
}
