import React, { useEffect, useState, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { detectConcordiumProvider } from '@concordium/browser-wallet-api-helpers';
import Connection from '../Connection';
import { version } from '../../package.json';

const VERIFIER_URL = '/api';

/**
 * The main component for the Application
 */
export default function Root() {
    const [account, setAccount] = useState<string>();
    const [authToken, setAuthToken] = useState<string>();
    const [isLinkHovered, setIsLinkHovered] = useState(false);
    const navigate = useNavigate();

    useEffect(() => {
        detectConcordiumProvider()
            .then((provider) => {
                // Listen for relevant events from the wallet.
                provider.on('accountChanged', (newAccount) => {
                    setAccount(newAccount);
                });
                
                provider.on(
                    'accountDisconnected',
                    () => void provider.getMostRecentlySelectedAccount().then(setAccount),
                );
                
                // Check if you are already connected
                provider.getMostRecentlySelectedAccount()
                    .then(setAccount)
                    .catch(console.error);
            })
            .catch(() => setAccount(undefined));
    }, []);

    const handleGoToDEX = useCallback(() => {
        navigate('/dex-page');
    }, [navigate]);

    return (
        <div style={{ 
            display: 'flex', 
            flexDirection: 'column',
            alignItems: 'center',
            minHeight: '100vh',
            backgroundImage: 'url("/concordium-home.jpg")',
            backgroundSize: 'cover',
            backgroundPosition: 'center',
            backgroundRepeat: 'no-repeat',
            color: 'white',
            padding: '20px'
        }}>
            <main style={{
                maxWidth: '600px',
                width: '100%',
                height: '100vh',
                textAlign: 'center',
                backgroundColor: 'rgba(0, 0, 0, 0.7)',
                borderRadius: '10px',
                padding: '2rem',
                backdropFilter: 'blur(5px)',
                display: 'flex',
                flexDirection: 'column',
                justifyContent: 'space-between'
            }}>
                <div>
                    <h1 style={{ 
                        fontSize: '2rem',
                        marginBottom: '2rem'
                    }}>
                        Concordium DEX
                    </h1>
                    <Connection
                        verifier={VERIFIER_URL}
                        account={account}
                        authToken={authToken}
                        setAccount={setAccount}
                        setAuthToken={setAuthToken}
                    />
                </div>

                {account && authToken && (
                    <div style={{ 
                        display: 'flex',
                        justifyContent: 'center',
                        alignItems: 'center',
                        flex: 1
                    }}>
                        <button 
                            onClick={handleGoToDEX}
                            style={{
                                padding: '15px 30px',
                                fontSize: '18px',
                                backgroundColor: '#308274',
                                color: 'white',
                                border: 'none',
                                borderRadius: '4px',
                                cursor: 'pointer',
                                transition: 'background-color 0.3s, transform 0.2s',
                                transform: 'scale(1)',
                                boxShadow: '0 4px 6px rgba(0, 0, 0, 0.1)'
                            }}
                            onMouseOver={(e) => {
                                e.currentTarget.style.backgroundColor = '#266A5E';
                                e.currentTarget.style.transform = 'scale(1.05)';
                            }}
                            onMouseOut={(e) => {
                                e.currentTarget.style.backgroundColor = '#308274';
                                e.currentTarget.style.transform = 'scale(1)';
                            }}
                        >
                            Go to DEX Page
                        </button>
                    </div>
                )}

                <div style={{ 
                    fontSize: '0.9rem',
                    display: 'flex',
                    justifyContent: 'center',
                    alignItems: 'center',
                    gap: '8px'
                }}>
                    Version: {version} |{' '}
                    <a
                        style={{ 
                            color: 'white',
                            textDecoration: isLinkHovered ? 'underline' : 'none',
                            cursor: 'pointer'
                        }}
                        href="#" // replace with published medium link
                        target="_blank"
                        rel="noreferrer"
                        onMouseEnter={() => setIsLinkHovered(true)}
                        onMouseLeave={() => setIsLinkHovered(false)}
                    >
                        Learn how to make a DEX like this
                    </a>
                </div>
            </main>
        </div>
    );
}