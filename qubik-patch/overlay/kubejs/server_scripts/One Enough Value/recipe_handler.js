let $RecipeType = Java.loadClass("net.minecraft.world.item.crafting.RecipeType");


// ==========================================================================
// 工具函数:安全地从任意对象中提取 ItemStack、数量和概率
// 返回 { stack, count, chance } 或 null(提取失败)
// ==========================================================================
function safeExtractItem(item) {
    if (!item) return null;

    let stack = null;
    let chance = 1.0;

    // 尝试 1: ChanceResult 等对象的 getStack() 方法
    try {
        if (typeof item.getStack === 'function') {
            stack = item.getStack();
        }
    } catch (e) { /* 忽略,尝试下一种 */ }

    // 尝试 2: 通过 .stack 属性访问
    if (!stack) {
        try {
            if (item.stack) stack = item.stack;
        } catch (e) { /* ignore */ }
    }

    // 尝试 3: 对象本身就是 ItemStack
    if (!stack) {
        try {
            if (typeof item.getCount === 'function' && typeof item.isEmpty === 'function') {
                stack = item;
            }
        } catch (e) { /* ignore */ }
    }

    if (!stack) return null;

    // 提取概率
    try {
        if (typeof item.getChance === 'function') {
            chance = item.getChance();
        } else if (typeof item.chance === 'number') {
            chance = item.chance;
        }
    } catch (e) { /* 保持默认 1.0 */ }

    // 校验并提取 count
    try {
        if (typeof stack.isEmpty !== 'function' || stack.isEmpty()) return null;
        if (typeof stack.getCount !== 'function') return null;

        let count = stack.getCount();
        if (count <= 0) return null;

        return { stack: stack, count: count, chance: chance };
    } catch (e) {
        return null;
    }
}

// 统一记录跳过的物品
function logSkippedItem(item, source, error) {
    try {
        let info = (item && item.getClass) ? item.getClass().getName() : typeof item;
        let value = 'n/a';
        try { value = String(item); } catch (ignored) {}
        let errMsg = (error && error.message) ? error.message : String(error);
        console.warn('[OneEnoughValue] Skipped in ' + source + ': ' + info + ' | ' + value + ' | ' + errMsg);
    } catch (e) { /* 日志失败就算了 */ }
}

// 安全地将 Iterable/List 转换为 JS 数组,避免在 forEach 里崩溃
function toArray(iterable) {
    let arr = [];
    if (!iterable) return arr;
    try {
        // 如果是 Java List
        if (typeof iterable.size === 'function' && typeof iterable.get === 'function') {
            let size = iterable.size();
            for (let i = 0; i < size; i++) {
                try { arr.push(iterable.get(i)); } catch (e) { /* skip bad element */ }
            }
            return arr;
        }
        // 如果已经是 JS 数组
        if (iterable.length !== undefined) {
            for (let i = 0; i < iterable.length; i++) {
                try { arr.push(iterable[i]); } catch (e) {}
            }
            return arr;
        }
        // 兜底:尝试 forEach 收集(最后手段)
        iterable.forEach(e => { try { arr.push(e); } catch (err) {} });
    } catch (e) {
        // 收集失败也返回已收集到的部分
    }
    return arr;
}


OEVEvents.addRecipeHandler(event => {
    let defaultMultiplier = global.DefaultRecipeValueMultiplier;

    event.getAllRecipeType().forEach(RecipeType => {
        let typeName = String(RecipeType.toString());
        let multiplier = global.RecipeValueMultiplierDict.get(typeName) ?? defaultMultiplier;

        event.addCustomRecipeHandler(RecipeType,
            // ===== 输入收集器 =====
            (recipe) => {
                let inputs = [];
                try {
                    let ingredients = recipe.getIngredients();
                    let ingArr = toArray(ingredients);
                    for (let i = 0; i < ingArr.length; i++) {
                        inputs.push(ingArr[i]);
                    }

                    // 序列组装配方
                    if (recipe.getSequence) {
                        try {
                            let startIngredient = recipe.getIngredient();
                            if (startIngredient) inputs.push(startIngredient);
                        } catch (e) {
                            logSkippedItem(recipe, 'inputGetter:startIngredient', e);
                        }

                        try {
                            let sequence = recipe.getSequence();
                            let seqArr = toArray(sequence);
                            for (let i = 0; i < seqArr.length; i++) {
                                let step = seqArr[i];
                                try {
                                    let stepRecipe = step.getRecipe();

                                    // 机械手配方不消耗手持物品时跳过
                                    if (stepRecipe.shouldKeepHeldItem && stepRecipe.shouldKeepHeldItem()) continue;

                                    let stepIngs = stepRecipe.getIngredients();
                                    if (stepIngs && stepIngs.size && stepIngs.size() == 2) {
                                        inputs.push(stepIngs.get(1));
                                    }
                                } catch (e) {
                                    logSkippedItem(step, 'inputGetter:sequenceStep', e);
                                }
                            }
                        } catch (e) {
                            logSkippedItem(recipe, 'inputGetter:sequence', e);
                        }
                    }
                } catch (e) {
                    logSkippedItem(recipe, 'inputGetter', e);
                }
                return inputs;
            },

            // ===== 输出收集器 =====
            (recipe, registryAccess) => {
                try {
                    let stack = recipe.getResultItem(registryAccess);
                    if (!stack || stack.isEmpty() || stack.getCount() === 0) return [];
                    return [stack];
                } catch (e) {
                    logSkippedItem(recipe, 'outputGetter', e);
                    return [];
                }
            },

            event.defaultRecipeExtraValueGetter,

            // ===== 价值设置器(核心修复区域)=====
            (recipe, stacks, totalValue, setter) => {
                try {
                    let currentTotalValue = totalValue * multiplier;

                    // -- 统一处理逻辑:计算未定价物品的价值分配 --
                    function calculateValueDistribution(items, state) {
                        let itemsArr = toArray(items);

                        for (let i = 0; i < itemsArr.length; i++) {
                            let item = itemsArr[i];

                            let extracted = null;
                            try {
                                extracted = safeExtractItem(item);
                            } catch (e) {
                                logSkippedItem(item, 'calculateValueDistribution:extract', e);
                                continue;
                            }

                            if (!extracted) continue;

                            try {
                                let itemId = String(extracted.stack.getItem().getId());

                                // 黑名单物品不参与价值分配
                                if (global.ValueBlackList.indexOf(itemId) !== -1) continue;

                                let expectedCount = extracted.count * extracted.chance;
                                state.itemCountMap[itemId] = (state.itemCountMap[itemId] || 0.0) + expectedCount;

                                let definedValue = global.FoodIngredientValueDict.get(itemId);
                                if (definedValue !== undefined) {
                                    state.consumedValue += definedValue * expectedCount;
                                } else {
                                    state.totalUnpricedCnt += expectedCount;
                                }
                            } catch (e) {
                                logSkippedItem(item, 'calculateValueDistribution:process', e);
                            }
                        }
                    }

                    // 1. 初始化状态
                    let state = {
                        itemCountMap: {},
                        totalUnpricedCnt: 0.0,
                        consumedValue: 0.0
                    };

                    // 2. 处理 stacks
                    calculateValueDistribution(stacks, state);

                    // 3. 处理 rollableResults(机械动力概率产出)
                    let rollableResults = null;
                    if (recipe.getRollableResults) {
                        try {
                            rollableResults = recipe.getRollableResults();
                            calculateValueDistribution(rollableResults, state);
                        } catch (e) {
                            logSkippedItem(recipe, 'valueSetter:getRollableResults', e);
                            rollableResults = null;
                        }
                    }

                    // 4. 计算未定价物品的单价
                    let valuePerUnpricedUnit = 1;
                    if (currentTotalValue > 0 && state.totalUnpricedCnt > 0) {
                        let remainingValue = currentTotalValue - state.consumedValue;
                        valuePerUnpricedUnit = Math.max(1, remainingValue / state.totalUnpricedCnt);
                    }

                    // 5a. 先收集 stacks 中已处理的 itemId(用于后续去重)
                    let processedIds = {};
                    let stacksArr = toArray(stacks);

                    // 5b. 给 stacks 里的未定价物品设置价值
                    for (let i = 0; i < stacksArr.length; i++) {
                        let stack = stacksArr[i];
                        try {
                            if (!stack || stack.isEmpty() || stack.getCount() <= 0) continue;

                            let itemId = String(stack.getItem().getId());
                            processedIds[itemId] = true;

                            if (global.ValueBlackList.indexOf(itemId) !== -1) continue;

                            if (global.FoodIngredientValueDict.get(itemId) === undefined) {
                                let stackValue = valuePerUnpricedUnit * stack.getCount();
                                setter.set(recipe, stack, stackValue);
                            }
                        } catch (e) {
                            logSkippedItem(stack, 'valueSetter:stacks', e);
                        }
                    }

                    // 5c. 给 rollableResults 里的未定价物品设置价值
                    if (rollableResults) {
                        let rollableArr = toArray(rollableResults);
                        for (let i = 0; i < rollableArr.length; i++) {
                            let result = rollableArr[i];

                            let extracted = null;
                            try {
                                extracted = safeExtractItem(result);
                            } catch (e) {
                                logSkippedItem(result, 'valueSetter:rollable:extract', e);
                                continue;
                            }

                            if (!extracted) continue;

                            try {
                                let itemId = String(extracted.stack.getItem().getId());

                                if (global.ValueBlackList.indexOf(itemId) !== -1) continue;

                                if (global.FoodIngredientValueDict.get(itemId) !== undefined) continue;

                                // 已在 stacks 中处理过则跳过
                                if (processedIds[itemId]) continue;

                                setter.set(recipe, Item.of(itemId), valuePerUnpricedUnit);
                            } catch (e) {
                                logSkippedItem(result, 'valueSetter:rollable:set', e);
                            }
                        }
                    }
                } catch (e) {
                    // 最外层兜底:单个配方处理失败不能影响整个游戏加载
                    logSkippedItem(recipe, 'valueSetter:fatal', e);
                }
            }
        );
    });
});
