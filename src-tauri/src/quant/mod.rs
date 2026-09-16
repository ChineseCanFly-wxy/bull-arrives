// src-tauri/src/quant/mod.rs
// 量化层：技术指标、因子打分、信号生成。
// 与「数据接入层（datasource）」解耦 —— 只吃纯数据（K 线序列），不碰网络。

pub mod backtest;
pub mod indicators;
pub mod levels;
pub mod playbook;
pub mod scorer;
