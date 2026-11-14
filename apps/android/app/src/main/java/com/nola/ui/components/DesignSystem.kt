package com.nola.ui.components

import android.os.Build
import android.view.HapticFeedbackConstants
import android.view.View
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.runtime.getValue
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.foundation.layout.RowScope
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.core.tween
import com.nola.R
import com.nola.ui.theme.NolaContinuousShape
import com.nola.ui.theme.nolaSurface

// ============================================================
// Design Tokens — matching iOS DesignSystem.swift
// ============================================================

object NolaColors {
    val chatBackground: Color @Composable get() = MaterialTheme.colorScheme.background
    val primary: Color @Composable get() = MaterialTheme.colorScheme.primary
    val primaryLight: Color @Composable get() = MaterialTheme.colorScheme.primaryContainer
    val accent: Color @Composable get() = MaterialTheme.colorScheme.primary
    val success: Color @Composable get() = if (MaterialTheme.colorScheme.background.luminance() < 0.5f) Color(0xFF7BC99B) else Color(0xFF237047)
    val warning: Color @Composable get() = MaterialTheme.colorScheme.tertiary
    val danger: Color @Composable get() = MaterialTheme.colorScheme.error

    val bgLight = Color(0xFFFFFFFF)
    val bgDark = Color(0xFF000000)
    val surfaceElevated: Color @Composable get() = MaterialTheme.colorScheme.surface
    val rowFill: Color @Composable get() = MaterialTheme.colorScheme.surfaceVariant
    val mutedFill: Color @Composable get() = MaterialTheme.colorScheme.surfaceContainer

    val borderLight: Color @Composable get() = MaterialTheme.colorScheme.outlineVariant
    val borderStrong: Color @Composable get() = MaterialTheme.colorScheme.outline
}

@JvmInline
value class NolaHaptics(private val view: View) {
    fun selection() {
        view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
    }

    fun primaryAction() {
        view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
    }

    fun success() {
        view.performHapticFeedback(
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM
            else HapticFeedbackConstants.VIRTUAL_KEY
        )
    }

    fun warning() {
        view.performHapticFeedback(
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.REJECT
            else HapticFeedbackConstants.LONG_PRESS
        )
    }
}

@Composable
fun rememberNolaHaptics(): NolaHaptics {
    val view = LocalView.current
    return remember(view) { NolaHaptics(view) }
}

object NolaSpacing {
    val xs = 4.dp
    val sm = 8.dp
    val md = 12.dp
    val lg = 16.dp
    val xl = 24.dp
    val xxl = 32.dp
}

object NolaRadius {
    val sm = 8.dp
    val md = 12.dp
    val lg = 12.dp
    val xl = 16.dp
    val pill = 999.dp
}

object NolaSize {
    val iconSmall = 28.dp
    val iconMedium = 32.dp
    val iconLarge = 40.dp
    val rowCompact = 48.dp
    val rowStandard = 56.dp
}

object NolaMotion {
    const val fast = 100
    const val base = 180
    const val slow = 220
    const val controlState = 160
    const val disclosure = 180
    const val streamReveal = 120
    const val messageEntrance = 190
    const val maximumVisualBacklog = 320
    const val followInterval = 16
    const val followPasses = 12
    const val reducedMotionFollowInterval = 16
    const val reducedMotionFollowPasses = 2
    val emphasizedEasing = CubicBezierEasing(0.22f, 1f, 0.36f, 1f)
    val disclosureEasing = CubicBezierEasing(0.20f, 0.74f, 0.24f, 1f)
    val streamRevealEasing = CubicBezierEasing(0.16f, 0.76f, 0.22f, 1f)
}

/** The same glyph rotates with the body instead of swapping at the first frame. */
@Composable
fun NolaDisclosureChevron(
    expanded: Boolean,
    reduceMotion: Boolean,
    modifier: Modifier = Modifier,
    contentDescription: String? = null,
) {
    val rotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
        label = "reply-disclosure-chevron",
    )
    Icon(Icons.Filled.ExpandMore, contentDescription = contentDescription,
        modifier = modifier.graphicsLayer { rotationZ = rotation },
        tint = MaterialTheme.colorScheme.onSurfaceVariant)
}

fun nolaDisclosureEnter(reduceMotion: Boolean): EnterTransition =
    fadeIn(
        tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        )
    ) + expandVertically(
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        ),
        expandFrom = Alignment.Top,
    )

fun nolaDisclosureExit(reduceMotion: Boolean): ExitTransition =
    fadeOut(
        tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        )
    ) + shrinkVertically(
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        ),
        shrinkTowards = Alignment.Top,
    )

// ============================================================
// Reusable Components — matching iOS equivalents
// ============================================================

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NolaTopAppBar(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    onBack: (() -> Unit)? = null,
    titleStyle: TextStyle? = null,
    actions: @Composable RowScope.() -> Unit = {}
) {
    TopAppBar(
        modifier = modifier,
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = MaterialTheme.colorScheme.background,
            titleContentColor = MaterialTheme.colorScheme.onBackground,
            navigationIconContentColor = MaterialTheme.colorScheme.primary,
            actionIconContentColor = MaterialTheme.colorScheme.primary
        ),
        title = {
            Column {
                Text(
                    title,
                    style = titleStyle ?: MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                if (!subtitle.isNullOrBlank()) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1
                    )
                }
            }
        },
        navigationIcon = {
            if (onBack != null) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.common_back))
                }
            }
        },
        actions = actions
    )
}

@Composable
fun nolaPageGutter(): Dp = if (LocalConfiguration.current.screenWidthDp < 360) 16.dp else 20.dp

@Composable
fun NolaPage(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit
) {
    val background = MaterialTheme.colorScheme.background
    Box(
        modifier = modifier
            .fillMaxSize()
            .background(background)
    ) {
        Column(
            modifier = Modifier
                .align(Alignment.TopCenter)
                .widthIn(max = 840.dp)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = nolaPageGutter())
                .padding(top = NolaSpacing.xl, bottom = 48.dp),
            verticalArrangement = Arrangement.spacedBy(NolaSpacing.xl),
            content = content
        )
    }
}

@Composable
fun NolaPageHeader(
    icon: ImageVector? = null,
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
    compactTrailing: Boolean = false,
    trailing: @Composable (() -> Unit)? = null
) {
    BoxWithConstraints(modifier.fillMaxWidth()) {
        val stackAction = !compactTrailing && trailing != null && (maxWidth < 360.dp || LocalDensity.current.fontScale >= 1.5f)
        Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                if (icon != null) {
                    Box(
                        modifier = Modifier
                            .size(46.dp)
                            .clip(NolaContinuousShape(NolaRadius.lg))
                            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.12f)),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(icon, null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                    }
                    Spacer(Modifier.width(NolaSpacing.md))
                }
                Column(modifier = Modifier.weight(1f)) {
                    Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
                    if (subtitle != null) {
                        Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (!stackAction) trailing?.invoke()
            }
            if (stackAction) {
                Box(Modifier.align(Alignment.End)) { trailing?.invoke() }
            }
        }
    }
}

@Composable
fun NolaCard(
    modifier: Modifier = Modifier,
    plain: Boolean = false,
    content: @Composable ColumnScope.() -> Unit
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .then(
                Modifier.nolaSurface(radius = NolaRadius.xl, raised = !plain)
                    .padding(NolaSpacing.lg)
            )
    ) {
        Column(
            verticalArrangement = Arrangement.spacedBy(NolaSpacing.md),
            content = content
        )
    }
}

@Composable
fun NolaCardCompact(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .nolaSurface(radius = NolaRadius.md)
            .padding(NolaSpacing.md)
    ) {
        Column(
            verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
            content = content
        )
    }
}

@Composable
fun NolaDivider(modifier: Modifier = Modifier) {
    androidx.compose.material3.HorizontalDivider(
        modifier = modifier.padding(vertical = NolaSpacing.md),
        thickness = 0.5.dp,
        color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f)
    )
}

@Composable
fun NolaBadge(
    text: String,
    icon: ImageVector? = null,
    tint: Color = MaterialTheme.colorScheme.primary,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NolaRadius.pill))
            .background(tint.copy(alpha = 0.12f))
            .padding(horizontal = 9.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (icon != null) {
            Icon(icon, null, Modifier.size(12.dp), tint = tint)
            Spacer(Modifier.width(4.dp))
        }
        Text(text, style = MaterialTheme.typography.labelSmall, color = tint)
    }
}

@Composable
fun NolaIconBadge(
    icon: ImageVector,
    modifier: Modifier = Modifier,
    size: Dp = NolaSize.iconSmall,
    tint: Color = MaterialTheme.colorScheme.primary,
    backgroundTint: Color = tint.copy(alpha = 0.12f)
) {
    Box(
        modifier = modifier
            .size(size)
            .clip(CircleShape)
            .background(backgroundTint),
        contentAlignment = Alignment.Center
    ) {
        Icon(
            icon,
            null,
            modifier = Modifier.size(size * 0.45f),
            tint = tint
        )
    }
}

@Composable
fun NolaSectionTitle(
    icon: ImageVector? = null,
    title: String,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (icon != null) {
            Icon(icon, null, Modifier.size(14.dp), tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.width(NolaSpacing.sm))
        }
        Text(
            title.uppercase(),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontWeight = FontWeight.Bold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f)
        )
        action?.invoke()
    }
}

@Composable
fun NolaEmptyState(
    icon: ImageVector,
    title: String,
    message: String? = null,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(NolaSpacing.xxl),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Surface(
            shape = NolaContinuousShape(NolaRadius.lg),
            color = MaterialTheme.colorScheme.primary.copy(alpha = 0.08f)
        ) {
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.padding(13.dp).size(24.dp),
                tint = MaterialTheme.colorScheme.primary.copy(alpha = 0.82f)
            )
        }
        Spacer(Modifier.height(NolaSpacing.md))
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.widthIn(max = 520.dp)
        )
        if (!message.isNullOrBlank()) {
            Spacer(Modifier.height(NolaSpacing.xs))
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.widthIn(max = 520.dp)
            )
        }
        if (action != null) {
            Spacer(Modifier.height(NolaSpacing.md))
            action()
        }
    }
}

@Composable
fun NolaCompactEmptyState(
    icon: ImageVector,
    title: String,
    message: String? = null,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Surface(
            shape = NolaContinuousShape(12.dp),
            color = MaterialTheme.colorScheme.primary.copy(alpha = 0.08f)
        ) {
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.padding(9.dp).size(20.dp),
                tint = MaterialTheme.colorScheme.primary
            )
        }
        Spacer(Modifier.height(NolaSpacing.sm))
        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center
        )
        if (!message.isNullOrBlank()) {
            Spacer(Modifier.height(NolaSpacing.xs))
            Text(
                message,
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        if (action != null) {
            Spacer(Modifier.height(NolaSpacing.md))
            action()
        }
    }
}

@Composable
fun NolaStatCard(
    title: String,
    value: String,
    icon: ImageVector? = null,
    modifier: Modifier = Modifier,
    tint: Color = MaterialTheme.colorScheme.primary
) {
    NolaCardCompact(modifier = modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (icon != null) {
                NolaIconBadge(icon = icon, tint = tint, backgroundTint = tint.copy(alpha = 0.12f))
                Spacer(Modifier.width(NolaSpacing.sm))
            }
            Column {
                Text(
                    value,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    color = tint
                )
                Text(
                    title,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
fun NolaChip(
    label: String,
    icon: ImageVector? = null,
    selected: Boolean = false,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    val bg = if (selected) MaterialTheme.colorScheme.primaryContainer
             else MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
    val fg = if (selected) MaterialTheme.colorScheme.onPrimaryContainer
             else MaterialTheme.colorScheme.onSurfaceVariant

    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NolaRadius.pill))
            .background(bg)
            .clickable(role = androidx.compose.ui.semantics.Role.Button) { onClick() }
            .sizeIn(minWidth = 48.dp, minHeight = 48.dp)
            .padding(horizontal = NolaSpacing.md, vertical = NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (icon != null) {
            Icon(icon, null, Modifier.size(14.dp), tint = fg)
            Spacer(Modifier.width(4.dp))
        }
        Text(label, style = MaterialTheme.typography.labelMedium, color = fg)
    }
}

@Composable
fun NolaHeroCard(
    icon: ImageVector,
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
    tint: Color = MaterialTheme.colorScheme.primary,
    metrics: List<Pair<String, String>> = emptyList(),
    content: @Composable (() -> Unit)? = null
) {
    NolaCard(modifier = modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            NolaIconBadge(icon = icon, size = NolaSize.iconMedium, tint = tint)
            Spacer(Modifier.width(NolaSpacing.sm))
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                if (subtitle != null) {
                    Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (metrics.isNotEmpty()) {
            Spacer(Modifier.height(NolaSpacing.md))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
            ) {
                metrics.forEach { (label, value) ->
                    NolaStatCard(
                        title = label,
                        value = value,
                        modifier = Modifier.weight(1f),
                        tint = tint
                    )
                }
            }
        }
        content?.invoke()
    }
}

@Composable
fun NolaProcessingPill(
    text: String,
    modifier: Modifier = Modifier,
    isError: Boolean = false
) {
    val bg = if (isError) NolaColors.danger.copy(alpha = 0.12f)
             else MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.6f)
    val fg = if (isError) NolaColors.danger
             else MaterialTheme.colorScheme.primary

    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NolaRadius.pill))
            .background(bg)
            .padding(horizontal = NolaSpacing.md, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (!isError) {
            androidx.compose.material3.CircularProgressIndicator(
                modifier = Modifier.size(12.dp),
                strokeWidth = 1.5.dp,
                color = fg
            )
            Spacer(Modifier.width(NolaSpacing.xs))
        }
        Text(text, style = MaterialTheme.typography.labelSmall, color = fg)
    }
}
