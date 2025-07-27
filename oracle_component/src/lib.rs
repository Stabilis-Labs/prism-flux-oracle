//! # Oracle Blueprint
//! Component verifying collateral prices.

use scrypto::prelude::*;

#[derive(ScryptoSbor, Clone)]
pub struct PriceMessage {
    pub market_id: String,
    pub price: Decimal,
    pub nonce: u64,
    pub created_at: u64,
}

#[blueprint]
mod oracle {
    enable_method_auth! {
        methods {
            update_price => PUBLIC;
            get_price => PUBLIC;
            update_lsu_multiplier => PUBLIC;
        }
    }

    const LSU_POOL: Global<LsuPool> = global_component!(
        LsuPool,
        "component_rdx1cppy08xgra5tv5melsjtj79c0ngvrlmzl8hhs7vwtzknp9xxs63mfp" //mainnet
        //"component_tdx_2_1cpdf8dsfslstthlvaa75kp652epw3pjn967dmf9kqhhzlger60mdn5" //stokenet dummy lsupool
    );

    extern_blueprint! {
        //"package_sim1pkgxxxxxxxxxpackgexxxxxxxxx000726633226xxxxxxxxxlk8hc9", //simulator package, uncomment to run tests
        //"package_tdx_2_1phrthm8neequrhdg8jxvvwd8xazccuaa8u3ufyemysade0ckv88an2", //stokenet morpher package
        "package_rdx1pka62r6e9754snp524ng3kfrkxma6qdxhzw86j7ka5nnl9m75nagmp", //mainnet morpher package
        MorpherOracle {
            fn check_price_input(&self, message: String, signature: String) -> PriceMessage;
        }

        // oracle address for stokenet: component_tdx_2_1cpt6kp3mqkds5uy858mqedwfglhsw25lhey59ev45ayce4yfsghf90
        // oracle address for mainnet: component_rdx1cpuqchky58ualnunh485cqne7p6dkepuwq0us2t5n89mz32k6pfppz
    }

    extern_blueprint! {
        "package_rdx1pkfrtmv980h85c9nvhxa7c9y0z4vxzt25c3gdzywz5l52g5t0hdeey", //mainnet lsu pool
        //"package_tdx_2_1ph6p4hk03a6p8f9mqzsfs9595jug6gmv2gxteggjtep3wv52t8g2ds",
        LsuPool {
            fn get_dex_valuation_xrd(&self) -> Decimal;
            fn get_liquidity_token_total_supply(&self) -> Decimal;
        }

        // lsu lp address: resource_rdx1thksg5ng70g9mmy9ne7wz0sc7auzrrwy7fmgcxzel2gvp8pj0xxfmf
    }

    struct Oracle {
        morpher_identifiers: HashMap<ResourceAddress, String>,
        oracle_address: ComponentAddress,
        max_price_age: i64,
        lsu_lp_address: ResourceAddress,
        lsu_multiplier: Decimal,
        last_lsu_multiplier_update: Instant,
        max_lsu_multiplier_age: i64,
        prices: HashMap<ResourceAddress, PriceEntry>,
    }

    impl Oracle {
        pub fn instantiate_oracle(
            owner_role: OwnerRole,
            oracle_address: ComponentAddress,
            dapp_def_address: GlobalAddress,
            lsu_lp_address: ResourceAddress,
        ) -> Global<Oracle> {
            let mut morpher_identifiers: HashMap<ResourceAddress, String> = HashMap::new();

            morpher_identifiers.insert(XRD, "GATEIO:XRD_USDT".to_string());
            morpher_identifiers.insert(lsu_lp_address, "GATEIO:XRD_USDT".to_string());

            Self {
                morpher_identifiers,
                oracle_address,
                max_price_age: 60,
                lsu_lp_address,
                lsu_multiplier: LSU_POOL.get_dex_valuation_xrd()
                    / LSU_POOL.get_liquidity_token_total_supply(),
                last_lsu_multiplier_update: Clock::current_time_rounded_to_seconds(),
                max_lsu_multiplier_age: 86400,
                prices: HashMap::new(),
            }
            .instantiate()
            .prepare_to_globalize(owner_role)
            .metadata(metadata! {
                init {
                    "name" => "Flux Oracle".to_string(), updatable;
                    "description" => "An oracle used to keep track of collateral prices for Flux".to_string(), updatable;
                    "info_url" => Url::of("https://flux.ilikeitstable.com"), updatable;
                    "dapp_definition" => dapp_def_address, updatable;
                }
            })
            .globalize()
        }

        pub fn update_price(
            &mut self,
            collateral: ResourceAddress,
            message: String,
            signature: String,
        ) {
            let morpher_oracle = Global::<MorpherOracle>::from(self.oracle_address);
            let price_message = morpher_oracle.check_price_input(message, signature);
            self.check_message_validity(collateral, price_message.clone());

            let now = Clock::current_time_rounded_to_seconds().seconds_since_unix_epoch;
            let price = if collateral == self.lsu_lp_address {
                self.check_for_lsu_multiplier_update();
                price_message.price * self.lsu_multiplier
            } else {
                price_message.price
            };

            self.prices.insert(
                collateral,
                PriceEntry {
                    price,
                    changed_at: now,
                    identifier: price_message.market_id,
                },
            );
        }

        pub fn get_price(&self, collateral: ResourceAddress) -> Decimal {
            let entry = self.prices.get(&collateral).expect("No price available for this collateral");
            let now = Clock::current_time_rounded_to_seconds().seconds_since_unix_epoch;
            if now > entry.changed_at + 60 {
                panic!("Price is too old");
            }
            entry.price
        }

        pub fn update_lsu_multiplier(&mut self) {
            let lsu_lp_supply = ResourceManager::from_address(self.lsu_lp_address).total_supply().unwrap();
            self.lsu_multiplier =
                LSU_POOL.get_dex_valuation_xrd() / lsu_lp_supply;
                
            self.last_lsu_multiplier_update = Clock::current_time_rounded_to_seconds();
        }

        fn check_for_lsu_multiplier_update(&mut self) {
            if Clock::current_time_is_strictly_after(
                self.last_lsu_multiplier_update
                    .add_seconds(self.max_lsu_multiplier_age)
                    .unwrap(),
                TimePrecision::Second,
            ) {
                self.update_lsu_multiplier();
            }
        }

        fn check_message_validity(&self, collateral: ResourceAddress, message: PriceMessage) {
            assert_eq!(
                *self
                    .morpher_identifiers
                    .get(&collateral)
                    .expect("Collateral not supported."),
                message.market_id
            );
            assert!(
                (message.created_at as i64 + self.max_price_age)
                    > Clock::current_time_rounded_to_seconds().seconds_since_unix_epoch
            )
        }
    }
}

#[derive(ScryptoSbor, Clone)]
pub struct PriceEntry {
    pub price: Decimal,
    pub changed_at: i64,
    pub identifier: String,
}
