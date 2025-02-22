/* eslint-disable no-console */
// Last updated: 2025-02-10 13:55:32 UTC by hoepeyemi
import esbuild, { BuildOptions } from 'esbuild';
import { htmlPlugin } from '@craftamap/esbuild-plugin-html';
import svgrPlugin from 'esbuild-plugin-svgr';
import fs from 'fs';

const watch = Boolean(process.env.WATCH);
const main = 'src/index.tsx';
const htmlTemplate = fs.readFileSync('src/index.html').toString();
const htmlOut = 'index.html';

const config: BuildOptions = {
    entryPoints: [main],
    entryNames: '[name]',
    bundle: true,
    minify: true,
    metafile: true,
    logLevel: 'info',
    sourcemap: 'inline',
    target: ['chrome67'],
    outdir: 'public',
    loader: {
        '.svg': 'dataurl' // Changed from 'file' to 'dataurl'
    },
    plugins: [
        htmlPlugin({
            files: [
                {
                    entryPoints: [main],
                    filename: htmlOut,
                    htmlTemplate,
                },
            ],
        }),
        svgrPlugin({
            typescript: true,
            exportType: 'default',
            ref: true,
            svgo: true,
            titleProp: true,
            svgoConfig: { // Added SVGO configuration
                plugins: [
                    {
                        name: 'preset-default',
                        params: {
                            overrides: {
                                removeViewBox: false
                            }
                        }
                    }
                ]
            }
        }),
    ],
};

if (watch) {
    config.watch = {
        onRebuild(error) {
            if (error) {
                console.error('watch build failed:', error);
                return;
            }
            console.log('rebuild successful');
        },
    };
}

esbuild
    .build(config)
    .then(() => {
        if (watch) {
            console.log('watching for changes...');
        }
    })
    .catch((error) => { // Added error handling
        console.error('Build failed:', error);
        process.exit(1);
    });