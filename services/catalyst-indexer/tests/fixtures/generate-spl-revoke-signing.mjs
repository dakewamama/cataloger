import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
    AccountRole, address, appendTransactionMessageInstructions, compileTransaction,
    createTransactionMessage, getAddressDecoder, getBase64EncodedWireTransaction,
    setTransactionMessageFeePayer, setTransactionMessageLifetimeUsingBlockhash,
} from '@solana/kit';

const fixtures = process.argv[2];
const snapshot = JSON.parse(readFileSync(join(fixtures, 'spl-snapshot.json')));
const capture = JSON.parse(readFileSync(join(fixtures, 'spl-revoke-rpc.json')));
const source = getAddressDecoder().decode(new Uint8Array(snapshot.targets[0].source));
const token = snapshot.accounts.find(([key]) =>
    getAddressDecoder().decode(new Uint8Array(key)) === source)[1];
const owner = getAddressDecoder().decode(new Uint8Array(token.data.slice(32, 64)));
const lifetime = capture.response.value.replacementBlockhash;
let message = createTransactionMessage({ version: 'legacy' });
message = setTransactionMessageFeePayer(owner, message);
message = setTransactionMessageLifetimeUsingBlockhash({
    blockhash: lifetime.blockhash,
    lastValidBlockHeight: BigInt(lifetime.lastValidBlockHeight),
}, message);
message = appendTransactionMessageInstructions([{
    programAddress: address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'),
    accounts: [
        { address: source, role: AccountRole.WRITABLE },
        { address: owner, role: AccountRole.READONLY_SIGNER },
    ],
    data: new Uint8Array([5]),
}], message);

console.log(JSON.stringify({
    generator: '@solana/kit@8.4.0',
    source,
    owner,
    blockhash: lifetime.blockhash,
    signing_transaction: getBase64EncodedWireTransaction(compileTransaction(message)),
}, null, 2));
