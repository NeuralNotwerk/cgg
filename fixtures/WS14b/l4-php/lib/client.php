<?php
class Client_Cache {
	public function get( $keys ) {
		$v = wp_cache_get_multiple( $keys );
		$w = only_fallback();
		return __( 'x' );
	}
}
