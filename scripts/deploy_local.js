import { deploy } from "./helpers.mjs";
import { LocalTerra } from "@terra-money/terra.js";

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
deploy(terra, wallet);



