import React from 'react';
import { BrowserRouter as Router, Routes, Route } from 'react-router-dom';
import { Network, WithWalletConnector, MAINNET, TESTNET } from '@concordium/react-components';
import Root from './components/Root';
import DEX from './components/DEX';
import { DEX_CONTRACT_INDEX_MAINNET, DEX_CONTRACT_INDEX_TESTNET } from '../src/components/constants';

export default function App() {
    const testnet = 'testnet';
    const mainnet = 'mainnet';

    let NETWORK: Network;
    let DEXContractIndex: bigint;


        NETWORK = TESTNET;
        DEXContractIndex = DEX_CONTRACT_INDEX_TESTNET;


    return (
        <Router>
            <Routes>
                <Route path="/" element={<Root />} />
                <Route 
                    path="/dex-page" 
                    element={
                        <WithWalletConnector network={NETWORK}>
                            {(props) => <DEX walletConnectionProps={props} DEXContractIndex={DEXContractIndex} />}
                        </WithWalletConnector>
                    } 
                />
            </Routes>
        </Router>
    );
}