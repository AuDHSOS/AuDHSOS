// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
// CLI transport fixture ONLY. This is not WPT and proves no conformance.
function setup(options) {}
function add_start_callback(callback) {print('__jrs_wpt_start__',false);}
let result_callback;
let complete_callback;
function add_result_callback(callback) { result_callback = callback; }
function add_completion_callback(callback) { complete_callback = callback; }
function done() { complete_callback([{status:0}], {status:0, message:null}); }
