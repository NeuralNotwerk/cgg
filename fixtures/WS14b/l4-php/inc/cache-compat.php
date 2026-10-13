<?php
if ( ! function_exists( 'wp_cache_get_multiple' ) ) :
	function wp_cache_get_multiple( $keys ) { return array(); }
endif;
if ( ! function_exists( 'only_fallback' ) ) {
	function only_fallback() { return 1; }
}
